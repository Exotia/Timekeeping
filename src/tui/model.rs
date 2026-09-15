//! The model owns all TUI state and draws every screen itself.

use std::time::{Duration, Instant};

use chrono::{Datelike, Days, Local, NaiveDate, NaiveTime, Timelike};
use tuirealm::application::Application;
use tuirealm::props::{AttrValue, Attribute};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout};
use tuirealm::terminal::{CrosstermTerminalAdapter, TerminalAdapter};

use super::components;
use super::ids::Id;
use super::msg::{
    Confirm, DayData, FormData, MonthData, Msg, RangeKind, StatsData, StoreCmd, StoreReply,
    UserEvent,
};
use super::theme::Theme;
use super::view::chrome;
use super::worker::Worker;
use crate::core::{
    Entry, HolidayCalendar, Minutes, Rules, check_overlap, deduction, minutes_of, parse_time,
};

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
    pub stats_range: RangeKind,
    pub form: Option<FormState>,
}

/// State of the open entry-form overlay: the raw field values plus the result of
/// validating them, recomputed on every keystroke.
pub struct FormState {
    pub data: FormData,
    pub error: Option<String>,
    /// `(gross of this entry, break deduction of the day, net of the day)`
    pub preview: Option<(Minutes, Minutes, Minutes)>,
}

/// Validate the raw form input against the day's other entries.
///
/// Returns `(start, end, gross of this entry, day deduction, day net)`.
pub fn validate_form(
    d: &FormData,
    existing: &[Entry],
    rules: &Rules,
) -> Result<(NaiveTime, NaiveTime, Minutes, Minutes, Minutes), String> {
    let start = parse_time(&d.start)
        .map_err(|_| format!("start: '{}' is not a time (try 800 or 8:00)", d.start))?;
    let end = parse_time(&d.end)
        .map_err(|_| format!("end: '{}' is not a time (try 1730 or 17:30)", d.end))?;
    // `Entry::interval` reads `end <= start` as crossing midnight and adds a day, so an
    // equal pair silently becomes a 24-hour entry that then overlaps everything else.
    // `end < start` is a genuine crossing and stays allowed.
    if minutes_of(end) == minutes_of(start) {
        return Err(
            "end: must differ from start (use a later time, or an earlier one to cross midnight)"
                .into(),
        );
    }
    if d.project.trim().is_empty() {
        return Err("project: required".into());
    }
    check_overlap(existing, start, end, d.id)
        .map_err(|_| "overlaps an existing entry".to_string())?;
    let this = Entry {
        id: d.id.unwrap_or(-1),
        date: existing.first().map(|e| e.date).unwrap_or_default(),
        start,
        end,
        project: String::new(),
        comment: String::new(),
    };
    let gross_this = this.duration();
    let gross_day: Minutes = existing
        .iter()
        .filter(|e| Some(e.id) != d.id)
        .map(Entry::duration)
        .sum::<Minutes>()
        + gross_this;
    let ded = deduction(gross_day, &rules.tiers);
    Ok((
        start,
        end,
        gross_this,
        ded,
        Minutes((gross_day - ded).0.max(0)),
    ))
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

    fn load_stats(&self) {
        let (from, to) = super::view::stats::range_for(self.stats_range, self.today);
        self.worker.send(StoreCmd::LoadStats { from, to });
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

    /// Mount a fresh form component over the day screen and give it focus.
    fn open_form(&mut self, data: FormData) {
        let projects: Vec<String> = self
            .day
            .as_ref()
            .map(|d| d.projects.iter().map(|p| p.name.clone()).collect())
            .unwrap_or_default();
        let _ = self.app.umount(&Id::Form);
        let _ = self.app.mount(
            Id::Form,
            Box::new(
                components::form::EntryForm::new(data.clone(), projects).with_theme(&self.theme),
            ),
            vec![],
        );
        self.form = Some(FormState {
            data,
            error: None,
            preview: None,
        });
        // A freshly opened blank form should not greet the user with a red error.
        self.refresh_form(false);
        self.focus(Id::Form);
    }

    fn close_form(&mut self) {
        self.form = None;
        let _ = self.app.umount(&Id::Form);
        self.focus_screen();
    }

    /// Re-validate the open form and push the footer line into the component.
    ///
    /// `show_error` is false while the form is untouched, so that opening an empty
    /// form does not immediately complain about the empty time fields.
    fn refresh_form(&mut self, show_error: bool) {
        let Some(form) = &self.form else { return };
        let entries = self
            .day
            .as_ref()
            .map(|d| d.day.entries.as_slice())
            .unwrap_or(&[]);
        let (error, preview) = match validate_form(&form.data, entries, &self.rules) {
            Ok((_, _, gross, ded, net)) => (None, Some((gross, ded, net))),
            Err(e) => (Some(e), None),
        };
        let error = if show_error { error } else { None };
        let (text, is_error) = match (&error, &preview) {
            (Some(e), _) => (e.clone(), true),
            (None, Some((g, d, n))) => (
                format!(
                    "gross +{} · break -{} · net +{}",
                    g.hhmm(),
                    d.hhmm(),
                    n.hhmm()
                ),
                false,
            ),
            _ => (String::new(), false),
        };
        if let Some(form) = &mut self.form {
            form.error = error;
            form.preview = preview;
        }
        let _ = self
            .app
            .attr(&Id::Form, Attribute::Text, AttrValue::String(text));
        let _ = self.app.attr(
            &Id::Form,
            Attribute::Custom(components::form::ERROR_FLAG),
            AttrValue::Flag(is_error),
        );
    }

    fn close_overlay(&mut self) {
        self.confirm = None;
        self.help = false;
        self.focus_screen();
    }

    pub fn update(&mut self, msg: Msg) {
        self.redraw = true;
        let is_day_kind_next = matches!(msg, Msg::DayKindNext);
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
                    let was_day = self.screen == Screen::Day;
                    self.screen = Screen::Month;
                    let _ = self.app.active(&Id::Month);
                    if was_day {
                        self.load_month();
                    }
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
                    Confirm::DeleteEntry(id) => {
                        self.worker.send(StoreCmd::DeleteEntry(id));
                    }
                    Confirm::SetKind(d, k) => {
                        self.worker.send(StoreCmd::SetKind(d, k));
                    }
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
            // --- statistics (Task 17) ---
            Msg::OpenStats => {
                self.screen = Screen::Stats;
                self.stats = None;
                self.focus(Id::Stats);
                self.load_stats();
            }
            Msg::StatsRange(r) => {
                self.stats_range = r;
                self.stats = None;
                self.load_stats();
            }
            // Filled in by Tasks 15–18.
            // --- day editor (Task 15) ---
            Msg::OpenDay => {
                self.screen = Screen::Day;
                self.day = None;
                self.day_cursor = 0;
                self.worker.send(StoreCmd::LoadDay(self.selected));
                self.focus(Id::Day);
            }
            Msg::DaySelect(n) => {
                let len = self.day.as_ref().map(|d| d.day.entries.len()).unwrap_or(0);
                if len > 0 {
                    self.day_cursor = (self.day_cursor as i32 + n).rem_euclid(len as i32) as usize;
                }
            }
            Msg::DayDelete => {
                if let Some(e) = self
                    .day
                    .as_ref()
                    .and_then(|d| d.day.entries.get(self.day_cursor))
                {
                    self.open_confirm(Confirm::DeleteEntry(e.id));
                }
            }
            Msg::DayKindNext | Msg::DayKindPrev => {
                let Some(d) = &self.day else { return };
                if !d.day.entries.is_empty() {
                    self.set_status("Day has entries; delete them first", true);
                    return;
                }
                if !crate::core::is_working_day(d.day.date) {
                    self.set_status("Weekends need no day type", true);
                    return;
                }
                let cur = super::view::day::kind_index(&d.day.kind) as i32;
                let next = (cur + if is_day_kind_next { 1 } else { -1 }).rem_euclid(6) as usize;
                let kind = crate::core::DayKind::parse(
                    super::view::day::KIND_CYCLE[next],
                    Some("absence"),
                )
                .unwrap();
                self.worker.send(StoreCmd::SetKind(d.day.date, kind));
            }
            // --- end day editor (Task 15) ---
            // --- entry form (Task 16) ---
            Msg::DayAdd => self.open_form(FormData::default()),
            Msg::DayEdit => {
                if let Some(e) = self
                    .day
                    .as_ref()
                    .and_then(|d| d.day.entries.get(self.day_cursor))
                {
                    let data = FormData {
                        id: Some(e.id),
                        start: e.start.format("%H:%M").to_string(),
                        end: e.end.format("%H:%M").to_string(),
                        project: e.project.clone(),
                        comment: e.comment.clone(),
                    };
                    self.open_form(data);
                }
            }
            Msg::FormCancel => self.close_form(),
            Msg::FormChanged(data) => {
                if let Some(form) = &mut self.form {
                    form.data = data;
                }
                self.refresh_form(true);
            }
            Msg::FormSubmit(data) => {
                let Some(day) = &self.day else { return };
                if !day.day.kind.allows_entries() {
                    self.set_status(
                        format!(
                            "{} day: change the day type to work first",
                            day.day.kind.display_name()
                        ),
                        true,
                    );
                    return;
                }
                match validate_form(&data, &day.day.entries, &self.rules) {
                    Err(_) => {
                        if let Some(form) = &mut self.form {
                            form.data = data;
                        }
                        self.refresh_form(true);
                    }
                    Ok((start, end, ..)) => {
                        let project = data.project.trim().to_string();
                        let comment = data.comment.trim().to_string();
                        match data.id {
                            None => {
                                self.worker.send(StoreCmd::AddEntry {
                                    date: day.day.date,
                                    start,
                                    end,
                                    project,
                                    comment,
                                });
                            }
                            Some(id) => {
                                self.worker.send(StoreCmd::UpdateEntry {
                                    id,
                                    start,
                                    end,
                                    project,
                                    comment,
                                });
                            }
                        }
                        self.close_form();
                    }
                }
            }
        }
    }

    fn on_store(&mut self, reply: StoreReply) {
        match reply {
            StoreReply::Month(m) => self.month = Some(m),
            StoreReply::Day(d) => {
                self.day_cursor = self.day_cursor.min(d.day.entries.len().saturating_sub(1));
                self.day = Some(d);
            }
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
        let form_open = self.form.is_some();
        let _ = term.draw(|f| {
            self.draw(f);
            if form_open {
                let area = f.area();
                self.app.view(&Id::Form, f, area);
            }
        });
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
            stats_range: RangeKind::ThisMonth,
            form: None,
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

    #[test]
    fn open_day_loads_and_focuses() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        m.update(Msg::OpenDay);
        assert_eq!(m.screen, Screen::Day);
        assert!(matches!(rx.try_recv().unwrap(), StoreCmd::LoadDay(dt) if dt == today));
        m.update(Msg::Back);
        assert_eq!(m.screen, Screen::Month);
    }

    #[test]
    fn day_delete_asks_confirm_for_selected_entry() {
        let today = d(2026, 9, 15);
        let (mut m, _rx) = model(today);
        let t = |h| chrono::NaiveTime::from_hms_opt(h, 0, 0).unwrap();
        let e = |id, s, e| crate::core::Entry {
            id,
            date: today,
            start: t(s),
            end: t(e),
            project: "A".into(),
            comment: "".into(),
        };
        m.screen = Screen::Day;
        m.day = Some(crate::tui::msg::DayData {
            day: Day {
                date: today,
                kind: DayKind::Work,
                entries: vec![e(1, 8, 9), e(2, 10, 11)],
            },
            projects: vec![],
        });
        m.update(Msg::DaySelect(1));
        m.update(Msg::DayDelete);
        assert_eq!(m.confirm, Some(Confirm::DeleteEntry(2)));
    }

    #[test]
    fn validate_form_cases() {
        let today = d(2026, 9, 15);
        let (m, _rx) = model(today);
        let t = |h, mi| chrono::NaiveTime::from_hms_opt(h, mi, 0).unwrap();
        let existing = vec![crate::core::Entry {
            id: 1,
            date: today,
            start: t(8, 0),
            end: t(12, 0),
            project: "A".into(),
            comment: "".into(),
        }];
        let fd = |s: &str, e: &str, p: &str| FormData {
            id: None,
            start: s.into(),
            end: e.into(),
            project: p.into(),
            comment: "".into(),
        };
        let ok = validate_form(&fd("1300", "16:00", "Alpha"), &existing, &m.rules).unwrap();
        assert_eq!(ok.0, t(13, 0));
        assert_eq!(ok.2, crate::core::Minutes(180)); // gross of this entry
        assert_eq!(ok.3, crate::core::Minutes(48)); // day deduction with 7h total
        assert_eq!(ok.4, crate::core::Minutes(420 - 48)); // day net
        assert!(
            validate_form(&fd("abc", "1600", "A"), &existing, &m.rules)
                .unwrap_err()
                .contains("start")
        );
        assert!(
            validate_form(&fd("1300", "", "A"), &existing, &m.rules)
                .unwrap_err()
                .contains("end")
        );
        assert!(
            validate_form(&fd("1300", "1600", " "), &existing, &m.rules)
                .unwrap_err()
                .contains("project")
        );
        assert!(
            validate_form(&fd("1100", "1300", "A"), &existing, &m.rules)
                .unwrap_err()
                .contains("overlap")
        );
        // A zero-length entry would be stretched to a full day by `Entry::interval`.
        assert!(
            validate_form(&fd("0900", "0900", "A"), &existing, &m.rules)
                .unwrap_err()
                .contains("end")
        );
        // Crossing midnight is still fine.
        let over = validate_form(&fd("2200", "0200", "A"), &existing, &m.rules).unwrap();
        assert_eq!(over.2, crate::core::Minutes(240));
        // editing entry 1 itself may overlap its old slot
        let mut edit = fd("0900", "1200", "A");
        edit.id = Some(1);
        assert!(validate_form(&edit, &existing, &m.rules).is_ok());
    }

    #[test]
    fn form_submit_sends_add_or_update() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        m.screen = Screen::Day;
        m.day = Some(crate::tui::msg::DayData {
            day: Day {
                date: today,
                kind: DayKind::Work,
                entries: vec![],
            },
            projects: vec![],
        });
        let t = |h, mi| NaiveTime::from_hms_opt(h, mi, 0).unwrap();
        // Surrounding whitespace must not reach the store.
        m.update(Msg::FormSubmit(FormData {
            id: None,
            start: "0900".into(),
            end: "1200".into(),
            project: " Alpha ".into(),
            comment: " x ".into(),
        }));
        assert!(matches!(
            rx.try_recv().unwrap(),
            StoreCmd::AddEntry { date, start, end, project, comment }
                if date == today
                    && start == t(9, 0)
                    && end == t(12, 0)
                    && project == "Alpha"
                    && comment == "x"
        ));
        m.update(Msg::FormSubmit(FormData {
            id: Some(7),
            start: "09:00".into(),
            end: "1200".into(),
            project: "Alpha".into(),
            comment: "x".into(),
        }));
        assert!(matches!(
            rx.try_recv().unwrap(),
            StoreCmd::UpdateEntry { id: 7, start, end, project, comment }
                if start == t(9, 0) && end == t(12, 0) && project == "Alpha" && comment == "x"
        ));
        m.update(Msg::DayAdd);
        assert!(m.form.is_some());
        m.update(Msg::FormSubmit(FormData {
            id: None,
            start: "zz".into(),
            end: "1200".into(),
            project: "Alpha".into(),
            comment: "".into(),
        }));
        assert!(rx.try_recv().is_err());
        assert!(m.form.as_ref().unwrap().error.is_some());
    }

    #[test]
    fn form_opens_prefilled_previews_and_cancels() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        let t = |h, mi| NaiveTime::from_hms_opt(h, mi, 0).unwrap();
        m.screen = Screen::Day;
        m.day = Some(crate::tui::msg::DayData {
            day: Day {
                date: today,
                kind: DayKind::Work,
                entries: vec![Entry {
                    id: 1,
                    date: today,
                    start: t(8, 0),
                    end: t(12, 0),
                    project: "Alpha".into(),
                    comment: "morning".into(),
                }],
            },
            projects: vec![crate::core::Project {
                id: 1,
                name: "Alpha".into(),
                color_index: 0,
                archived: false,
            }],
        });

        // `e` edits the entry under the cursor, prefilled.
        m.update(Msg::DayEdit);
        let f = m.form.as_ref().expect("form open");
        assert_eq!(f.data.id, Some(1));
        assert_eq!(f.data.start, "08:00");
        assert_eq!(f.data.end, "12:00");
        assert_eq!(f.data.project, "Alpha");
        assert_eq!(f.data.comment, "morning");
        // Editing the entry against itself is valid, so the footer previews it.
        assert_eq!(f.error, None);
        // 4h gross crosses the first break tier: 18 minutes off, 3:42 net.
        assert_eq!(f.preview, Some((Minutes(240), Minutes(18), Minutes(222))));

        // A live edit that overlaps the entry's own old slot is fine; a bad time is not.
        m.update(Msg::FormChanged(FormData {
            start: "abc".into(),
            ..m.form.as_ref().unwrap().data.clone()
        }));
        let f = m.form.as_ref().unwrap();
        assert!(f.error.as_deref().unwrap().contains("start"));
        assert_eq!(f.preview, None);

        // Esc closes the overlay and hands focus back to the day screen.
        m.update(Msg::FormCancel);
        assert!(m.form.is_none());

        // `a` opens a blank form; a new entry clashing with the existing one is refused.
        m.update(Msg::DayAdd);
        assert_eq!(m.form.as_ref().unwrap().data, FormData::default());
        // Nothing typed yet, so no complaint about the empty fields.
        assert_eq!(m.form.as_ref().unwrap().error, None);
        m.update(Msg::FormChanged(FormData {
            id: None,
            start: "1100".into(),
            end: "1300".into(),
            project: "Beta".into(),
            comment: String::new(),
        }));
        assert!(
            m.form
                .as_ref()
                .unwrap()
                .error
                .as_deref()
                .unwrap()
                .contains("overlap")
        );
        m.update(Msg::FormSubmit(m.form.as_ref().unwrap().data.clone()));
        assert!(rx.try_recv().is_err());
        assert!(m.form.is_some(), "an invalid submit keeps the form open");

        // A valid slot after the existing entry goes through and closes the form.
        m.update(Msg::FormSubmit(FormData {
            id: None,
            start: "1300".into(),
            end: "1600".into(),
            project: "Beta".into(),
            comment: String::new(),
        }));
        assert!(matches!(
            rx.try_recv().unwrap(),
            StoreCmd::AddEntry { project, .. } if project == "Beta"
        ));
        assert!(m.form.is_none());
    }

    #[test]
    fn form_submit_refused_on_a_non_work_day() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        m.screen = Screen::Day;
        m.day = Some(crate::tui::msg::DayData {
            day: Day {
                date: today,
                kind: DayKind::Vacation,
                entries: vec![],
            },
            projects: vec![],
        });
        m.update(Msg::FormSubmit(FormData {
            id: None,
            start: "0900".into(),
            end: "1200".into(),
            project: "Alpha".into(),
            comment: String::new(),
        }));
        assert!(rx.try_recv().is_err());
        assert!(m.status.as_ref().unwrap().1, "shown as an error");
    }

    #[test]
    fn day_kind_cycle_sends_set_kind() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        m.screen = Screen::Day;
        m.day = Some(crate::tui::msg::DayData {
            day: Day {
                date: today,
                kind: DayKind::Work,
                entries: vec![],
            },
            projects: vec![],
        });
        m.update(Msg::DayKindNext);
        assert!(matches!(
            rx.try_recv().unwrap(),
            StoreCmd::SetKind(_, DayKind::Vacation)
        ));
        m.day.as_mut().unwrap().day.kind = DayKind::Vacation;
        m.update(Msg::DayKindPrev);
        assert!(matches!(
            rx.try_recv().unwrap(),
            StoreCmd::SetKind(_, DayKind::Work)
        ));
    }
}
