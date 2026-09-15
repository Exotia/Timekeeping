//! The model owns all TUI state and draws every screen itself.

use std::time::{Duration, Instant};

use chrono::{Datelike, Days, Local, NaiveDate, NaiveTime};
use tuirealm::application::Application;
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout};
use tuirealm::terminal::{CrosstermTerminalAdapter, TerminalAdapter};

use super::ids::Id;
use super::msg::{Confirm, DayData, MonthData, Msg, StatsData, StoreCmd, StoreReply, UserEvent};
use super::theme::Theme;
use super::view::chrome;
use super::worker::Worker;
use crate::core::{HolidayCalendar, Rules};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Month,
    Day,
    Stats,
}

pub struct Model {
    pub app: Application<Id, Msg, UserEvent>,
    pub terminal: Option<CrosstermTerminalAdapter>,
    pub theme: Theme,
    pub rules: Rules,
    pub cal: HolidayCalendar,
    pub vacation_allowance: u32,
    pub screen: Screen,
    pub selected: NaiveDate,
    pub today: NaiveDate,
    pub now: NaiveTime,
    pub month: Option<MonthData>,
    pub day: Option<DayData>,
    pub stats: Option<StatsData>,
    pub day_cursor: usize,
    pub confirm: Option<Confirm>,
    pub help: bool,
    pub status: Option<(String, bool, Instant)>,
    pub quit: bool,
    pub redraw: bool,
    pub worker: Worker,
    pub size: (u16, u16),
}

pub const STATUS_TTL: Duration = Duration::from_secs(5);

impl Model {
    pub fn set_status(&mut self, msg: impl Into<String>, is_error: bool) {
        self.status = Some((msg.into(), is_error, Instant::now()));
    }

    pub fn load_month(&self) {
        self.worker.send(StoreCmd::LoadMonth {
            year: self.selected.year(),
            month: self.selected.month(),
        });
    }

    pub fn update(&mut self, msg: Msg) {
        self.redraw = true;
        match msg {
            Msg::Quit => self.quit = true,
            Msg::Tick => {
                let now = Local::now().naive_local();
                if now.date() != self.today {
                    self.today = now.date();
                    self.load_month();
                }
                self.now = now.time();
                if let Some((_, _, at)) = &self.status
                    && at.elapsed() > STATUS_TTL
                {
                    self.status = None;
                }
            }
            Msg::Error(e) => self.set_status(e, true),
            Msg::Info(i) => self.set_status(i, false),
            Msg::Store(reply) => self.on_store(reply),
            Msg::SelectDay(n) => {
                let d = if n >= 0 {
                    self.selected.checked_add_days(Days::new(n as u64))
                } else {
                    self.selected.checked_sub_days(Days::new((-n) as u64))
                };
                if let Some(d) = d {
                    let changed_month =
                        (d.year(), d.month()) != (self.selected.year(), self.selected.month());
                    self.selected = d;
                    if changed_month {
                        self.load_month();
                    }
                }
            }
            Msg::SelectMonth(n) => {
                let (y, m) = (self.selected.year(), self.selected.month() as i32 - 1 + n);
                let (y, m) = (y + m.div_euclid(12), (m.rem_euclid(12) + 1) as u32);
                let last = super::worker::month_range(y, m).1;
                self.selected =
                    NaiveDate::from_ymd_opt(y, m, self.selected.day().min(last.day())).unwrap();
                self.load_month();
            }
            Msg::GoToday => {
                self.selected = self.today;
                self.load_month();
            }
            Msg::ToggleHelp => self.help = !self.help,
            Msg::Back => {
                if self.help {
                    self.help = false;
                } else if self.confirm.is_some() {
                    self.confirm = None;
                } else {
                    self.screen = Screen::Month;
                    let _ = self.app.active(&Id::Month);
                }
            }
            // Filled in by Tasks 15–18.
            _ => {}
        }
    }

    fn on_store(&mut self, reply: StoreReply) {
        match reply {
            StoreReply::Month(m) => self.month = Some(m),
            StoreReply::Day(d) => self.day = Some(d),
            StoreReply::Stats(s) => self.stats = Some(s),
            StoreReply::Changed(msg) => {
                if !msg.is_empty() {
                    self.set_status(msg, false);
                }
                self.load_month();
                if self.screen == Screen::Day {
                    self.worker.send(StoreCmd::LoadDay(self.selected));
                }
            }
            StoreReply::Failed(e) => self.set_status(e, true),
        }
    }

    pub fn view(&mut self) {
        let mut term = self.terminal.take().expect("terminal");
        let _ = term.draw(|f| self.draw(f));
        self.terminal = Some(term);
    }

    /// Pure over `&self`: used by snapshot tests through a `TestBackend`.
    pub fn draw(&self, f: &mut Frame) {
        let area = f.area();
        if area.width < 80 || area.height < 24 {
            chrome::draw_too_small(f, area, &self.theme, (area.width, area.height));
            return;
        }
        let [title, body, status, hints] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(5),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(area);
        chrome::draw_title_bar(f, title, &self.theme, &self.title_info());
        match self.screen {
            Screen::Month => super::view::month::draw(self, f, body),
            Screen::Day => super::view::day::draw(self, f, body),
            Screen::Stats => super::view::stats::draw(self, f, body),
        }
        chrome::draw_status_bar(
            f,
            status,
            &self.theme,
            self.status.as_ref().map(|(m, e, _)| (m.as_str(), *e)),
        );
        chrome::draw_key_hints(f, hints, &self.theme, self.key_hints());
        if let Some(c) = &self.confirm {
            chrome::draw_confirm(f, area, &self.theme, &self.confirm_text(c));
        }
        if self.help {
            chrome::draw_help(f, area, &self.theme, "Keys", self.help_keys());
        }
    }

    pub fn title_info(&self) -> chrome::TitleInfo {
        let m = self.month.as_ref();
        chrome::TitleInfo {
            title: format!(
                "{} {}",
                month_name(self.selected.month()).to_uppercase(),
                self.selected.year()
            ),
            balance: m.map(|m| m.balance_total).unwrap_or_default(),
            clock: m.and_then(|m| m.session.as_ref()).map(|s| {
                (
                    s.start.format("%H:%M").to_string(),
                    crate::core::Minutes((self.now - s.start).num_minutes().max(0) as i32),
                )
            }),
            vacation: m.map(|m| (m.vacation_used_this_year, self.vacation_allowance)),
        }
    }

    pub fn key_hints(&self) -> &'static [(&'static str, &'static str)] {
        match self.screen {
            Screen::Month => &[
                ("↑↓", "day"),
                ("[ ]", "month"),
                ("⏎", "edit"),
                ("i/o", "clock"),
                ("s", "stats"),
                ("v f x p", "day type"),
                ("?", "help"),
                ("q", "quit"),
            ],
            Screen::Day => &[
                ("↑↓", "entry"),
                ("a", "add"),
                ("e", "edit"),
                ("d", "delete"),
                ("←→", "day type"),
                ("Esc", "back"),
            ],
            Screen::Stats => &[
                ("1", "month"),
                ("2", "last month"),
                ("3", "quarter"),
                ("4", "year"),
                ("Esc", "back"),
            ],
        }
    }

    pub fn help_keys(&self) -> &'static [(&'static str, &'static str)] {
        match self.screen {
            Screen::Month => &[
                ("↑ ↓ j k", "move by day"),
                ("[ ]", "previous / next month"),
                ("t", "jump to today"),
                ("Enter", "open day editor"),
                ("s", "statistics"),
                ("i / o", "clock in / out"),
                ("v", "vacation"),
                ("f", "flex day"),
                ("x", "sick"),
                ("p", "public holiday"),
                ("w", "reset to work day"),
                ("q", "quit"),
            ],
            Screen::Day => &[
                ("↑ ↓", "select entry"),
                ("a", "add entry"),
                ("e", "edit entry"),
                ("d", "delete entry"),
                ("← →", "change day type"),
                ("Esc", "back"),
            ],
            Screen::Stats => &[("1 2 3 4", "range"), ("Esc", "back")],
        }
    }

    pub fn confirm_text(&self, c: &Confirm) -> String {
        match c {
            Confirm::DeleteEntry(_) => "Delete this entry?".into(),
            Confirm::SetKind(d, k) => format!("Set {d} to {}?", k.display_name().to_lowercase()),
            Confirm::ClockInReplace => "Already clocked in. Replace?".into(),
        }
    }
}

pub fn month_name(m: u32) -> &'static str {
    [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ][(m as usize).saturating_sub(1).min(11)]
}
