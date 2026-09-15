//! The model owns all TUI state and draws every screen itself.

use std::time::{Duration, Instant};

use chrono::{Datelike, Days, Local, NaiveDate, NaiveTime, Timelike};
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

    pub fn focus(&mut self, id: Id) {
        let _ = self.app.active(&id);
    }

    pub fn focus_screen(&mut self) {
        let id = match self.screen {
            Screen::Month => Id::Month,
            Screen::Day => Id::Day,
            Screen::Stats => Id::Stats,
        };
        self.focus(id);
    }

    fn open_confirm(&mut self, c: Confirm) {
        self.confirm = Some(c);
        self.focus(Id::Confirm);
    }

    fn close_overlay(&mut self) {
        self.confirm = None;
        self.help = false;
        self.focus_screen();
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
            Msg::Back => {
                if self.help || self.confirm.is_some() {
                    self.close_overlay();
                } else {
                    self.screen = Screen::Month;
                    let _ = self.app.active(&Id::Month);
                }
            }
            Msg::ClockIn => {
                if !crate::core::is_working_day(self.today) {
                    self.set_status("Cannot clock in on a weekend", true);
                    return;
                }
                if self.month.as_ref().is_some_and(|m| m.session.is_some()) {
                    self.open_confirm(Confirm::ClockInReplace);
                    return;
                }
                self.worker.send(StoreCmd::ClockIn(
                    self.today,
                    self.now.with_second(0).unwrap(),
                ));
            }
            Msg::ClockOut => {
                if !self.month.as_ref().is_some_and(|m| m.session.is_some()) {
                    self.set_status("Not clocked in", true);
                    return;
                }
                self.worker.send(StoreCmd::ClockOut {
                    project: None,
                    comment: String::new(),
                });
            }
            Msg::SetKind(kind) => {
                let has_entries = self
                    .month
                    .as_ref()
                    .and_then(|m| m.days.iter().find(|d| d.date == self.selected))
                    .is_some_and(|d| !d.entries.is_empty());
                if has_entries && kind != crate::core::DayKind::Work {
                    self.set_status("Day has entries; delete them first", true);
                    return;
                }
                if !crate::core::is_working_day(self.selected) && kind != crate::core::DayKind::Work
                {
                    self.set_status("Weekends need no day type", true);
                    return;
                }
                self.open_confirm(Confirm::SetKind(self.selected, kind));
            }
            Msg::AskConfirm(c) => self.open_confirm(c),
            Msg::ConfirmNo => self.close_overlay(),
            Msg::ConfirmYes => {
                let Some(c) = self.confirm.take() else {
                    return;
                };
                self.close_overlay();
                match c {
                    Confirm::DeleteEntry(id) => self.worker.send(StoreCmd::DeleteEntry(id)),
                    Confirm::SetKind(d, k) => self.worker.send(StoreCmd::SetKind(d, k)),
                    Confirm::ClockInReplace => {
                        // Replacing discards the running session rather than recording it.
                        self.worker.send(StoreCmd::ClearSession);
                        self.worker.send(StoreCmd::ClockIn(
                            self.today,
                            self.now.with_second(0).unwrap(),
                        ));
                    }
                }
            }
            Msg::ToggleHelp => {
                self.help = !self.help;
                if self.help {
                    self.focus(Id::Help)
                } else {
                    self.focus_screen()
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

/// Test-only model/data constructors, exposed (not `cfg(test)`-gated) so the
/// integration tests in `tests/` can build a `Model` without a terminal.
#[doc(hidden)]
pub mod testing {
    use super::*;
    use crate::tui::msg::StoreCmd;
    use std::sync::mpsc::{Receiver, channel};

    pub fn model(today: NaiveDate) -> (Model, Receiver<StoreCmd>) {
        let (tx, rx) = channel();
        let app: Application<Id, Msg, UserEvent> =
            Application::init(tuirealm::listener::EventListenerCfg::default());
        let m = Model {
            app,
            terminal: None,
            theme: Theme::dark(),
            rules: Rules {
                daily_target: crate::core::Minutes(468),
                tiers: crate::core::default_tiers(),
                start_date: today,
                initial_balance: crate::core::Minutes::ZERO,
            },
            cal: HolidayCalendar::default(),
            vacation_allowance: 30,
            screen: Screen::Month,
            selected: today,
            today,
            now: NaiveTime::from_hms_opt(10, 0, 0).unwrap(),
            month: None,
            day: None,
            stats: None,
            day_cursor: 0,
            confirm: None,
            help: false,
            status: None,
            quit: false,
            redraw: false,
            worker: Worker { tx },
            size: (100, 30),
        };
        (m, rx)
    }

    pub fn month_data(
        today: NaiveDate,
        days: Vec<crate::core::Day>,
        session: Option<crate::store::Session>,
    ) -> MonthData {
        MonthData {
            year: today.year(),
            month: today.month(),
            days,
            balance_before: Default::default(),
            balance_total: Default::default(),
            session,
            projects: vec![],
            vacation_used_this_year: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use crate::core::{Day, DayKind};
    use crate::tui::msg::StoreCmd;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, dd: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, dd).unwrap()
    }

    #[test]
    fn set_kind_asks_confirm_then_sends_cmd() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        m.month = Some(month_data(
            today,
            vec![Day {
                date: today,
                kind: DayKind::Work,
                entries: vec![],
            }],
            None,
        ));
        m.update(Msg::SetKind(DayKind::Vacation));
        assert_eq!(m.confirm, Some(Confirm::SetKind(today, DayKind::Vacation)));
        m.update(Msg::ConfirmYes);
        assert_eq!(m.confirm, None);
        assert!(
            matches!(rx.try_recv().unwrap(), StoreCmd::SetKind(dt, DayKind::Vacation) if dt == today)
        );
    }

    #[test]
    fn set_kind_on_day_with_entries_is_refused() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        let e = crate::core::Entry {
            id: 1,
            date: today,
            start: chrono::NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            end: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            project: "A".into(),
            comment: "".into(),
        };
        m.month = Some(month_data(
            today,
            vec![Day {
                date: today,
                kind: DayKind::Work,
                entries: vec![e],
            }],
            None,
        ));
        m.update(Msg::SetKind(DayKind::Sick));
        assert!(m.confirm.is_none());
        assert!(m.status.as_ref().unwrap().1);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn clock_in_out_flow() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        m.month = Some(month_data(today, vec![], None));
        m.update(Msg::ClockIn);
        assert!(matches!(rx.try_recv().unwrap(), StoreCmd::ClockIn(dt, _) if dt == today));
        m.update(Msg::ClockOut);
        assert!(m.status.as_ref().unwrap().1); // not clocked in → error status
        let sess = crate::store::Session {
            date: today,
            start: chrono::NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            project: None,
        };
        m.month = Some(month_data(today, vec![], Some(sess)));
        m.update(Msg::ClockIn);
        assert_eq!(m.confirm, Some(Confirm::ClockInReplace));
        m.update(Msg::ConfirmNo);
        assert!(m.confirm.is_none());
        m.update(Msg::ClockOut);
        assert!(matches!(rx.try_recv().unwrap(), StoreCmd::ClockOut { .. }));
    }

    #[test]
    fn clock_in_on_weekend_is_refused() {
        let sat = d(2026, 9, 19);
        let (mut m, rx) = model(sat);
        m.month = Some(month_data(sat, vec![], None));
        m.update(Msg::ClockIn);
        assert!(rx.try_recv().is_err());
        assert!(m.status.as_ref().unwrap().1);
    }

    #[test]
    fn navigation_and_month_change() {
        let today = d(2026, 1, 31);
        let (mut m, rx) = model(today);
        m.update(Msg::SelectMonth(1));
        assert_eq!(m.selected, d(2026, 2, 28));
        assert!(matches!(
            rx.try_recv().unwrap(),
            StoreCmd::LoadMonth {
                year: 2026,
                month: 2
            }
        ));
        m.update(Msg::SelectMonth(-2));
        assert_eq!(m.selected, d(2025, 12, 28));
        m.update(Msg::GoToday);
        assert_eq!(m.selected, today);
        m.update(Msg::SelectDay(1));
        assert_eq!(m.selected, d(2026, 2, 1));
    }
}
