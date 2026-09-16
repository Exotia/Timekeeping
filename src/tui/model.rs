//! The model owns all TUI state and draws every screen itself.

use std::path::PathBuf;
use std::str::FromStr;
use std::time::{Duration, Instant};

use chrono::{Datelike, Days, Local, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use tuirealm::application::Application;
use tuirealm::props::{AttrValue, Attribute};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout};
use tuirealm::terminal::{CrosstermTerminalAdapter, TerminalAdapter};

use super::components;
use super::ids::Id;
use super::msg::{
    ChartMode, Confirm, DayData, FormData, MonthData, Msg, RangeKind, SettingsData, StatsData,
    StoreCmd, StoreReply, UserEvent,
};
use super::theme::Theme;
use super::view::chrome;
use super::worker::Worker;
use crate::config::{Config, ConfigPatch};
use crate::core::{
    Entry, HolidayCalendar, HoursFormat, Minutes, Rules, check_overlap, check_range, parse_date,
    parse_time, session_deduction,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Month,
    Day,
    Stats,
}

pub struct Model {
    pub app: Application<Id, Msg, UserEvent>,
    /// The data directory, so the settings overlay can rewrite `config.toml`.
    pub home: PathBuf,
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
    pub stats_range: RangeKind,
    pub form: Option<FormState>,
    pub settings: Option<SettingsState>,
    pub clock_picker: Option<ClockPickerState>,
    /// How every duration on screen is spelled; `u` flips it and writes it back
    /// to `config.toml`.
    pub hours: HoursFormat,
    /// The date the statistics range is measured around: the screen shows the
    /// `stats_range` period this day falls in. `[` and `]` move it by whole
    /// periods, `t` brings it back to today.
    pub stats_anchor: NaiveDate,
    /// What the statistics chart's bars measure; `r` flips it. It belongs to the
    /// session, not to a range: walking to another period keeps the view the
    /// reader asked for.
    pub chart_mode: ChartMode,
}

/// State of the open clock-in overlay: whether submitting it switches the running
/// session to another project, or opens the first one.
pub struct ClockPickerState {
    pub switching: bool,
}

/// State of the open entry-form overlay: the raw field values plus the result of
/// validating them, recomputed on every keystroke.
pub struct FormState {
    pub data: FormData,
    pub error: Option<String>,
    /// `(gross of this entry, break deduction of the day, net of the day)`
    pub preview: Option<(Minutes, Minutes, Minutes)>,
}

/// State of the open settings overlay: the raw field values and, once the user
/// has touched them, why they are not acceptable yet.
pub struct SettingsState {
    pub data: SettingsData,
    pub error: Option<String>,
}

/// Turn the raw settings fields into a [`ConfigPatch`], naming the field that is
/// wrong. The wording matches the entry form: `"field: what is wrong"`.
pub fn validate_settings(d: &SettingsData, today: NaiveDate) -> Result<ConfigPatch, String> {
    let start = parse_date(&d.start, today).map_err(|_| {
        format!(
            "start: '{}' is not a date (try 2026-09-15 or today)",
            d.start
        )
    })?;
    let balance = Minutes::from_str(d.balance.trim())
        .map_err(|_| format!("balance: '{}' is not ±HH:MM (try +12:30)", d.balance))?;
    let target = Minutes::from_str(d.target.trim())
        .map_err(|_| format!("target: '{}' is not HH:MM (try 07:48)", d.target))?;
    if target.0 <= 0 {
        return Err("target: must be greater than 0".to_string());
    }
    let vacation: u32 = d
        .vacation
        .trim()
        .parse()
        .map_err(|_| format!("vacation: '{}' is not a number of days", d.vacation))?;
    Ok(ConfigPatch {
        start_date: Some(start),
        initial_balance_minutes: Some(balance.0),
        daily_target_minutes: Some(target.0),
        vacation_days_per_year: Some(vacation),
        // The overlay has no field for it; `u` owns that key.
        hours_format: None,
    })
}

/// The footer line of a settings overlay that validates: what saving would set.
pub fn settings_footer(patch: &ConfigPatch, f: HoursFormat) -> String {
    format!(
        "balance {} · target {} · vacation {}",
        Minutes(patch.initial_balance_minutes.unwrap_or(0)).fmt_signed(f),
        Minutes(patch.daily_target_minutes.unwrap_or(0)).fmt_unsigned(f),
        patch.vacation_days_per_year.unwrap_or(0)
    )
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
    // One shared rule with the parser and the store; only the wording is the form's,
    // because the form names the field that is wrong.
    check_range(start, end).map_err(|_| {
        "end: must differ from start (use a later time, or an earlier one to cross midnight)"
            .to_string()
    })?;
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
        break_share: None,
    };
    let gross_this = this.duration();
    // The day as saving this form would leave it: every other entry plus this
    // one. The entry under edit is taken out, so its old slot neither counts
    // twice nor pretends to be the neighbour of its own new one.
    let day: Vec<Entry> = existing
        .iter()
        .filter(|e| Some(e.id) != d.id)
        .cloned()
        .chain(std::iter::once(this))
        .collect();
    let gross_day: Minutes = day.iter().map(Entry::duration).sum();
    let ivs: Vec<(i32, i32)> = day.iter().map(Entry::interval).collect();
    let ded = session_deduction(&ivs, &rules.tiers);
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

    /// Queue a command for the store thread, saying so in the status bar if that
    /// thread is gone — otherwise the UI would keep accepting keys that do nothing.
    pub fn send(&mut self, cmd: StoreCmd) {
        if !self.worker.send(cmd) {
            self.set_status("store thread stopped — quit with q and restart", true);
        }
    }

    pub fn load_month(&mut self) {
        self.send(StoreCmd::LoadMonth {
            year: self.selected.year(),
            month: self.selected.month(),
        });
    }

    fn load_stats(&mut self) {
        let (from, to) = super::view::stats::range_for(self.stats_range, self.stats_anchor);
        self.send(StoreCmd::LoadStats {
            from,
            to,
            year: self.stats_anchor.year(),
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

    /// Mount a fresh settings overlay, prefilled from the rules this session is
    /// running with, and give it focus.
    fn open_settings(&mut self) {
        // The two durations stay ±HH:MM whatever `u` says: these are the values in
        // the text fields, and `validate_settings` reads them back with
        // `Minutes::from_str`, which is the same format `--balance` and `--target`
        // take on the command line. Only the footer below them follows the setting.
        let data = SettingsData {
            start: self.rules.start_date.to_string(),
            balance: self.rules.initial_balance.to_string(),
            target: self.rules.daily_target.hhmm(),
            vacation: self.vacation_allowance.to_string(),
        };
        let _ = self.app.umount(&Id::Settings);
        let _ = self.app.mount(
            Id::Settings,
            Box::new(components::settings::SettingsForm::new(data.clone()).with_theme(&self.theme)),
            vec![],
        );
        self.settings = Some(SettingsState { data, error: None });
        self.refresh_settings(false);
        self.focus(Id::Settings);
    }

    fn close_settings(&mut self) {
        self.settings = None;
        let _ = self.app.umount(&Id::Settings);
        self.focus_screen();
    }

    /// The projects the clock-in overlay offers: the last used one first, so it is
    /// what Enter takes, and — when switching — without the one already running.
    fn picker_projects(&self, current: Option<&str>) -> Vec<String> {
        let Some(m) = self.month.as_ref() else {
            return Vec::new();
        };
        let mut names: Vec<String> = m
            .projects
            .iter()
            .map(|p| p.name.clone())
            .filter(|n| Some(n.as_str()) != current)
            .collect();
        // An archived project can still be the last used one, so it is put in front
        // rather than looked up in the list.
        if let Some(last) = m
            .last_used_project
            .as_ref()
            .filter(|l| Some(l.as_str()) != current)
        {
            names.retain(|n| n != last);
            names.insert(0, last.clone());
        }
        names
    }

    /// Mount a fresh clock-in overlay over the month screen and give it focus.
    fn open_clock_picker(&mut self, title: String, projects: Vec<String>, switching: bool) {
        let _ = self.app.umount(&Id::ClockPicker);
        let _ = self.app.mount(
            Id::ClockPicker,
            Box::new(
                components::project_picker::ProjectPicker::new(title, projects)
                    .with_theme(&self.theme),
            ),
            vec![],
        );
        self.clock_picker = Some(ClockPickerState { switching });
        self.focus(Id::ClockPicker);
    }

    fn close_clock_picker(&mut self) {
        self.clock_picker = None;
        let _ = self.app.umount(&Id::ClockPicker);
        self.focus_screen();
    }

    /// Re-validate the open settings overlay and push its footer line back in.
    ///
    /// `show_error` is false right after opening, where the values come straight
    /// from the running config and there is nothing to complain about yet.
    fn refresh_settings(&mut self, show_error: bool) {
        let Some(s) = &self.settings else { return };
        let (error, preview) = match validate_settings(&s.data, self.today) {
            Ok(patch) => (None, Some(settings_footer(&patch, self.hours))),
            Err(e) => (Some(e), None),
        };
        let error = if show_error { error } else { None };
        let (text, is_error) = match (&error, &preview) {
            (Some(e), _) => (e.clone(), true),
            (None, Some(p)) => (p.clone(), false),
            _ => (String::new(), false),
        };
        if let Some(s) = &mut self.settings {
            s.error = error;
        }
        self.set_settings_footer(text, is_error);
    }

    /// Push a footer line into the settings overlay.
    fn set_settings_footer(&mut self, text: String, is_error: bool) {
        let _ = self
            .app
            .attr(&Id::Settings, Attribute::Text, AttrValue::String(text));
        let _ = self.app.attr(
            &Id::Settings,
            Attribute::Custom(components::form::ERROR_FLAG),
            AttrValue::Flag(is_error),
        );
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
                    g.fmt_unsigned(self.hours),
                    d.fmt_unsigned(self.hours),
                    n.fmt_unsigned(self.hours)
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
                self.selected = super::view::stats::add_months(self.selected, n);
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
            Msg::OpenClockPicker => {
                // No weekday condition: weekend work counts towards the balance, and
                // the CLI (`tk in`) has always allowed it.
                let session = self.month.as_ref().and_then(|m| m.session.clone());
                let current = session.as_ref().and_then(|s| s.project.clone());
                let title = match (session.is_some(), current.as_deref()) {
                    (true, Some(p)) => format!("Switch project — currently {p}"),
                    (true, None) => "Switch project".to_string(),
                    _ => "Clock in — project".to_string(),
                };
                let projects = self.picker_projects(current.as_deref());
                self.open_clock_picker(title, projects, session.is_some());
            }
            // The overlay only moved its highlight; every `update` repaints anyway.
            Msg::ClockPickerChanged => {}
            Msg::ClockPickerCancel => self.close_clock_picker(),
            Msg::ClockPickerSubmit(name) => {
                let Some(state) = self.clock_picker.take() else {
                    return;
                };
                self.close_clock_picker();
                let name = name.trim().to_string();
                if name.is_empty() {
                    self.set_status("Pick a project first", true);
                    return;
                }
                if state.switching {
                    self.send(StoreCmd::Switch { project: name });
                } else {
                    self.send(StoreCmd::ClockIn(
                        self.today,
                        self.now.with_second(0).unwrap(),
                        name,
                    ));
                }
            }
            Msg::ClockOut => {
                if !self.month.as_ref().is_some_and(|m| m.session.is_some()) {
                    self.set_status("Not clocked in", true);
                    return;
                }
                self.send(StoreCmd::ClockOut {
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
                        self.send(StoreCmd::DeleteEntry(id));
                    }
                    Confirm::SetKind(d, k) => {
                        self.send(StoreCmd::SetKind(d, k));
                    }
                }
            }
            Msg::ToggleHours => {
                self.hours = self.hours.toggle();
                let patch = ConfigPatch {
                    hours_format: Some(self.hours),
                    ..Default::default()
                };
                // The toggle is what the user pressed for; a home that cannot be
                // written costs them the setting next time, not this screen.
                match Config::write_updates(&self.home, &patch) {
                    Ok(_) => self.set_status(
                        match self.hours {
                            HoursFormat::Hm => "Hours shown as h:mm",
                            HoursFormat::Decimal => "Hours shown as decimal",
                        },
                        false,
                    ),
                    Err(e) => self.set_status(format!("hours format not saved: {e}"), true),
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
            // --- settings overlay ---
            Msg::OpenSettings => self.open_settings(),
            Msg::SettingsChanged(data) => {
                if let Some(s) = &mut self.settings {
                    s.data = data;
                }
                self.refresh_settings(true);
            }
            Msg::SettingsCancel => self.close_settings(),
            Msg::SettingsSubmit(data) => {
                if let Some(s) = &mut self.settings {
                    s.data = data.clone();
                }
                let patch = match validate_settings(&data, self.today) {
                    Ok(p) => p,
                    Err(_) => {
                        self.refresh_settings(true);
                        return;
                    }
                };
                match Config::write_updates(&self.home, &patch) {
                    Ok(cfg) => {
                        self.rules = cfg.rules();
                        self.vacation_allowance = cfg.vacation_days_per_year;
                        // Every balance on screen was computed from the old rules.
                        self.load_month();
                        self.set_status("Settings saved", false);
                        self.close_settings();
                    }
                    Err(e) => {
                        // Writing failed (a read-only home, a hand-edited file that no
                        // longer parses): say so in the footer and stay open.
                        let text = e.to_string();
                        if let Some(s) = &mut self.settings {
                            s.error = Some(text.clone());
                        }
                        self.set_settings_footer(text, true);
                    }
                }
            }
            // --- statistics (Task 17) ---
            Msg::OpenStats => {
                self.screen = Screen::Stats;
                self.stats = None;
                // The screen always opens on the period being lived in, however
                // far the last visit had walked away from it.
                self.stats_anchor = self.today;
                self.focus(Id::Stats);
                self.load_stats();
            }
            Msg::StatsRange(r) => {
                // The anchor stays put: switching from the week of 15 August to
                // the month shows August, not this month.
                self.stats_range = r;
                self.stats = None;
                self.load_stats();
            }
            Msg::StatsShift(n) => {
                self.stats_anchor =
                    super::view::stats::shift_anchor(self.stats_range, self.stats_anchor, n);
                self.stats = None;
                self.load_stats();
            }
            Msg::StatsToday => {
                self.stats_anchor = self.today;
                self.stats = None;
                self.load_stats();
            }
            // Nothing has to be loaded again: both modes are drawn from the
            // range that is already in hand.
            Msg::ToggleChartMode => {
                self.chart_mode = match self.chart_mode {
                    ChartMode::PerPeriod => ChartMode::Running,
                    ChartMode::Running => ChartMode::PerPeriod,
                };
            }
            // Filled in by Tasks 15–18.
            // --- day editor (Task 15) ---
            Msg::OpenDay => {
                self.screen = Screen::Day;
                self.day = None;
                self.day_cursor = 0;
                self.send(StoreCmd::LoadDay(self.selected));
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
                self.send(StoreCmd::SetKind(d.day.date, kind));
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
                                self.send(StoreCmd::AddEntry {
                                    date: day.day.date,
                                    start,
                                    end,
                                    project,
                                    comment,
                                });
                            }
                            Some(id) => {
                                self.send(StoreCmd::UpdateEntry {
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
                    self.send(StoreCmd::LoadDay(self.selected));
                }
            }
            StoreReply::Failed(e) => self.set_status(e, true),
        }
    }

    pub fn view(&mut self) {
        let mut term = self.terminal.take().expect("terminal");
        let form_open = self.form.is_some();
        let settings_open = self.settings.is_some();
        let picker_open = self.clock_picker.is_some();
        let _ = term.draw(|f| {
            self.draw(f);
            let area = f.area();
            if form_open {
                self.app.view(&Id::Form, f, area);
            }
            if settings_open {
                self.app.view(&Id::Settings, f, area);
            }
            if picker_open {
                self.app.view(&Id::ClockPicker, f, area);
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
        chrome::draw_title_bar(f, title, &self.theme, &self.title_info(), self.hours);
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
        chrome::draw_key_hints(f, hints, &self.theme, self.key_hints_for(area.width));
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
                // Both ends as date-times: a session opened yesterday keeps counting
                // instead of freezing at 00:00 when the clock passes midnight.
                (
                    s.project.clone().unwrap_or_default(),
                    s.start.format("%H:%M").to_string(),
                    crate::core::running_minutes(
                        NaiveDateTime::new(s.date, s.start),
                        NaiveDateTime::new(self.today, self.now),
                    ),
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
                ("c", "settings"),
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
                ("u", "units"),
                ("Esc", "back"),
            ],
            Screen::Stats => &[
                ("1-4", "range"),
                ("[ ]", "shift"),
                ("t", "today"),
                ("r", "running"),
                ("u", "units"),
                ("Esc", "back"),
            ],
        }
    }

    /// The hints that fit into `width`. The row is a single line, and the month
    /// screen's full set is wider than the 80-column minimum; there it gives up the
    /// day-type keys, which are the group the `?` help spells out most fully.
    ///
    /// `u` is not in either month set: adding it to the reduced one takes the row
    /// to 83 columns, past the 80 the UI promises to work at. `?` lists it.
    pub fn key_hints_for(&self, width: u16) -> &'static [(&'static str, &'static str)] {
        let full = self.key_hints();
        if chrome::hints_width(full) <= width {
            return full;
        }
        match self.screen {
            Screen::Month => &[
                ("↑↓", "day"),
                ("[ ]", "month"),
                ("⏎", "edit"),
                ("i/o", "clock"),
                ("s", "stats"),
                ("c", "settings"),
                ("?", "help"),
                ("q", "quit"),
            ],
            _ => full,
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
                ("c", "settings"),
                ("i", "clock in / switch project"),
                ("o", "clock out"),
                ("v", "vacation"),
                ("f", "flex day"),
                ("x", "sick"),
                ("p", "public holiday"),
                ("w", "reset to work day"),
                ("u", "toggle h:mm / decimal hours"),
                ("q", "quit"),
            ],
            Screen::Day => &[
                ("↑ ↓", "select entry"),
                ("a", "add entry"),
                ("e", "edit entry"),
                ("d", "delete entry"),
                ("← →", "change day type"),
                ("u", "toggle h:mm / decimal hours"),
                ("Esc", "back"),
            ],
            Screen::Stats => &[
                ("1", "week (Monday … Sunday)"),
                ("2", "calendar month"),
                ("3", "calendar quarter"),
                ("4", "calendar year"),
                ("[ ]", "previous / next period"),
                ("PgUp PgDn", "previous / next period"),
                ("t", "back to today"),
                ("r", "toggle per-period / running balance"),
                ("u", "toggle h:mm / decimal hours"),
                ("Esc", "back"),
            ],
        }
    }

    pub fn confirm_text(&self, c: &Confirm) -> String {
        match c {
            Confirm::DeleteEntry(_) => "Delete this entry?".into(),
            Confirm::SetKind(d, k) => format!("Set {d} to {}?", k.display_name().to_lowercase()),
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
            // Nothing writes here unless a test opens the settings overlay; a test
            // that saves settings points `home` at a `tempfile::tempdir()` of its own.
            home: std::env::temp_dir().join("tk-testing-model"),
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
            stats_range: RangeKind::Month,
            form: None,
            settings: None,
            clock_picker: None,
            hours: HoursFormat::Hm,
            stats_anchor: today,
            chart_mode: ChartMode::default(),
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
            last_used_project: None,
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
    use tuirealm::component::AppComponent;

    fn d(y: i32, m: u32, dd: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, dd).unwrap()
    }

    #[test]
    fn toggle_hours_flips_the_format_and_writes_it_to_the_config() {
        let today = d(2026, 9, 15);
        let (mut m, _rx) = model(today);
        let dir = tempfile::tempdir().unwrap();
        m.home = dir.path().join("h");
        assert_eq!(m.hours, HoursFormat::Hm);

        m.update(Msg::ToggleHours);
        assert_eq!(m.hours, HoursFormat::Decimal);
        let written = std::fs::read_to_string(m.home.join("config.toml")).unwrap();
        assert!(written.contains("hours_format = \"decimal\""), "{written}");
        let (msg, is_error, _) = m.status.as_ref().unwrap();
        assert_eq!(msg, "Hours shown as decimal");
        assert!(!is_error);

        m.update(Msg::ToggleHours);
        assert_eq!(m.hours, HoursFormat::Hm);
        let written = std::fs::read_to_string(m.home.join("config.toml")).unwrap();
        assert!(written.contains("hours_format = \"hm\""), "{written}");
        assert_eq!(m.status.as_ref().unwrap().0, "Hours shown as h:mm");
    }

    #[test]
    fn toggle_hours_keeps_the_toggle_when_the_config_cannot_be_written() {
        let today = d(2026, 9, 15);
        let (mut m, _rx) = model(today);
        // A file stands where the home directory would have to be created.
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("not-a-dir");
        std::fs::write(&blocker, "x").unwrap();
        m.home = blocker.join("h");
        m.update(Msg::ToggleHours);
        // The session still gets what it asked for; only the file stayed behind.
        assert_eq!(m.hours, HoursFormat::Decimal);
        assert!(m.status.as_ref().unwrap().1, "{:?}", m.status);
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
            break_share: None,
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

    /// A month with the given projects known and `last` used most recently.
    fn month_with_projects(
        today: NaiveDate,
        names: &[&str],
        last: Option<&str>,
        session: Option<crate::store::Session>,
    ) -> MonthData {
        let mut m = month_data(today, vec![], session);
        m.projects = names
            .iter()
            .enumerate()
            .map(|(i, n)| crate::core::Project {
                id: i as i64 + 1,
                name: (*n).to_string(),
                color_index: i as u8,
                archived: false,
            })
            .collect();
        m.last_used_project = last.map(str::to_string);
        m
    }

    #[test]
    fn clock_in_picks_a_project_then_clocks_in() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        m.month = Some(month_with_projects(
            today,
            &["Beta", "Alpha"],
            Some("Alpha"),
            None,
        ));
        m.update(Msg::OpenClockPicker);
        let st = m.clock_picker.as_ref().expect("the picker is open");
        assert!(!st.switching, "nothing is running yet");
        assert!(m.app.mounted(&Id::ClockPicker));
        // The last used project is offered first, so Enter takes it.
        assert_eq!(
            m.picker_projects(None),
            vec!["Alpha".to_string(), "Beta".into()]
        );
        m.update(Msg::ClockPickerSubmit("Alpha".into()));
        assert!(m.clock_picker.is_none(), "the overlay closes on submit");
        assert!(matches!(
            rx.try_recv().unwrap(),
            StoreCmd::ClockIn(dt, _, p) if dt == today && p == "Alpha"
        ));

        // Nothing running: `o` only complains.
        m.update(Msg::ClockOut);
        assert!(m.status.as_ref().unwrap().1);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn clock_in_while_running_switches_the_project() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        let sess = crate::store::Session {
            date: today,
            start: chrono::NaiveTime::from_hms_opt(8, 0, 0).unwrap(),
            project: Some("Alpha".into()),
            state: crate::store::SessionState::Working,
        };
        m.month = Some(month_with_projects(
            today,
            &["Alpha", "Beta"],
            Some("Alpha"),
            Some(sess),
        ));
        m.update(Msg::OpenClockPicker);
        assert!(m.clock_picker.as_ref().expect("open").switching);
        // The project already running is not on offer.
        assert_eq!(m.picker_projects(Some("Alpha")), vec!["Beta".to_string()]);
        m.update(Msg::ClockPickerSubmit("Beta".into()));
        assert!(matches!(
            rx.try_recv().unwrap(),
            StoreCmd::Switch { project } if project == "Beta"
        ));

        // Esc closes the overlay without sending anything.
        m.update(Msg::OpenClockPicker);
        assert!(m.clock_picker.is_some());
        m.update(Msg::ClockPickerCancel);
        assert!(m.clock_picker.is_none());
        assert!(!m.app.mounted(&Id::ClockPicker));
        assert!(rx.try_recv().is_err());

        // `o` books the running session on its own project.
        m.update(Msg::ClockOut);
        assert!(matches!(
            rx.try_recv().unwrap(),
            StoreCmd::ClockOut { project: None, .. }
        ));
    }

    #[test]
    fn clock_in_on_weekend_is_allowed() {
        // Weekend work counts positively towards the balance, so nothing refuses it.
        let sat = d(2026, 9, 19);
        let (mut m, rx) = model(sat);
        m.month = Some(month_with_projects(sat, &["Alpha"], Some("Alpha"), None));
        m.update(Msg::OpenClockPicker);
        m.update(Msg::ClockPickerSubmit("Alpha".into()));
        assert!(matches!(rx.try_recv().unwrap(), StoreCmd::ClockIn(dt, _, _) if dt == sat));
        assert!(m.status.is_none(), "no complaint in the status bar");
    }

    #[test]
    fn a_dead_store_thread_is_reported_in_the_status_bar() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        drop(rx); // the store thread is gone
        m.update(Msg::GoToday);
        let (msg, is_error, _) = m.status.as_ref().expect("a status message");
        assert!(is_error, "shown as an error");
        assert!(msg.contains("store thread stopped"), "{msg}");
    }

    #[test]
    fn a_session_from_yesterday_keeps_counting_past_midnight() {
        let today = d(2026, 9, 15);
        let (mut m, _rx) = model(today);
        m.now = NaiveTime::from_hms_opt(1, 0, 0).unwrap();
        m.month = Some(month_data(
            today,
            vec![],
            Some(crate::store::Session {
                date: d(2026, 9, 14),
                start: NaiveTime::from_hms_opt(23, 0, 0).unwrap(),
                project: Some("Alpha".into()),
                state: crate::store::SessionState::Working,
            }),
        ));
        let info = m.title_info();
        assert_eq!(
            info.clock,
            Some((
                "Alpha".to_string(),
                "23:00".to_string(),
                crate::core::Minutes(120)
            ))
        );
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

    /// The statistics hints name every key the screen has, inside 80 columns.
    #[test]
    fn the_statistics_hints_name_the_navigation_keys() {
        let (mut m, _rx) = model(d(2026, 9, 15));
        m.screen = Screen::Stats;
        let hints = m.key_hints();
        for pair in [
            ("1-4", "range"),
            ("[ ]", "shift"),
            ("t", "today"),
            ("r", "running"),
            ("u", "units"),
            ("Esc", "back"),
        ] {
            assert!(hints.contains(&pair), "{pair:?} missing from {hints:?}");
        }
        // The whole set fits the minimum terminal, so nothing has to be trimmed.
        assert_eq!(m.key_hints_for(80), hints);
        let help = m.help_keys();
        for key in ["[ ]", "t", "r"] {
            assert!(
                help.iter().any(|(k, _)| *k == key),
                "{key} missing from the help"
            );
        }
    }

    /// `r` flips the chart between the per-period bars and the running balance.
    /// The mode is the reader's for the session: walking to another period, to
    /// another range length, or away from the screen and back keeps it.
    #[test]
    fn r_toggles_the_chart_mode_and_navigation_keeps_it() {
        let today = d(2026, 9, 15);
        let (mut m, _rx) = model(today);
        m.update(Msg::OpenStats);
        assert_eq!(m.chart_mode, ChartMode::PerPeriod);
        m.redraw = false;
        m.update(Msg::ToggleChartMode);
        assert_eq!(m.chart_mode, ChartMode::Running);
        assert!(m.redraw, "the chart has to be repainted");
        for msg in [
            Msg::StatsRange(RangeKind::Week),
            Msg::StatsShift(-1),
            Msg::StatsToday,
            Msg::Back,
            Msg::OpenStats,
        ] {
            m.update(msg.clone());
            assert_eq!(m.chart_mode, ChartMode::Running, "lost after {msg:?}");
        }
        m.update(Msg::ToggleChartMode);
        assert_eq!(m.chart_mode, ChartMode::PerPeriod);
    }

    /// `[`, `]` and `t` walk the statistics screen through whole periods, and the
    /// range that is loaded is always the one the anchor falls in.
    #[test]
    fn statistics_walk_through_periods() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        let loaded = |rx: &std::sync::mpsc::Receiver<StoreCmd>| match rx.try_recv().unwrap() {
            StoreCmd::LoadStats { from, to, year } => (from, to, year),
            other => panic!("expected LoadStats, got {other:?}"),
        };
        m.update(Msg::OpenStats);
        assert_eq!(m.stats_anchor, today);
        assert_eq!(loaded(&rx), (d(2026, 9, 1), d(2026, 9, 30), 2026));

        // One month back: the previous calendar month, not a window of 30 days.
        m.update(Msg::StatsShift(-1));
        assert_eq!(m.stats_anchor, d(2026, 8, 15));
        assert_eq!(loaded(&rx), (d(2026, 8, 1), d(2026, 8, 31), 2026));

        // Switching the range keeps where the user has navigated to: Saturday
        // 15 August sits in the week Mon 10 … Sun 16.
        m.update(Msg::StatsRange(RangeKind::Week));
        assert_eq!(m.stats_anchor, d(2026, 8, 15));
        assert_eq!(loaded(&rx), (d(2026, 8, 10), d(2026, 8, 16), 2026));

        // Two weeks on is fourteen days on.
        m.update(Msg::StatsShift(1));
        m.update(Msg::StatsShift(1));
        assert_eq!(m.stats_anchor, d(2026, 8, 29));
        let _ = loaded(&rx);
        assert_eq!(loaded(&rx), (d(2026, 8, 24), d(2026, 8, 30), 2026));

        // `t` comes back to the period today falls in.
        m.update(Msg::StatsToday);
        assert_eq!(m.stats_anchor, today);
        assert_eq!(loaded(&rx), (d(2026, 9, 14), d(2026, 9, 20), 2026));

        // A year back asks for that year's vacation, so the allowance line is
        // about the year on screen.
        m.update(Msg::StatsRange(RangeKind::Year));
        assert_eq!(loaded(&rx), (d(2026, 1, 1), d(2026, 12, 31), 2026));
        m.update(Msg::StatsShift(-1));
        assert_eq!(loaded(&rx), (d(2025, 1, 1), d(2025, 12, 31), 2025));

        // Re-opening the screen starts at today again.
        m.update(Msg::Back);
        let _ = rx.try_recv();
        m.update(Msg::OpenStats);
        assert_eq!(m.stats_anchor, today);
        assert_eq!(loaded(&rx), (d(2026, 1, 1), d(2026, 12, 31), 2026));
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
            break_share: None,
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
            break_share: None,
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
        // The hour between 12:00 and the new entry splits the day: the 4:00
        // morning loses 18, the three-hour afternoon nothing — the same figures
        // the day screen will show.
        assert_eq!(ok.3, crate::core::Minutes(18)); // day deduction
        assert_eq!(ok.4, crate::core::Minutes(420 - 18)); // day net
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
    fn validate_form_previews_the_session_deduction() {
        let today = d(2026, 9, 15);
        let (m, _rx) = model(today);
        let t = |h, mi| chrono::NaiveTime::from_hms_opt(h, mi, 0).unwrap();
        let e = |id, s, en| crate::core::Entry {
            id,
            date: today,
            start: s,
            end: en,
            project: "A".into(),
            comment: String::new(),
            break_share: None,
        };
        let fd = |id, s: &str, en: &str| FormData {
            id,
            start: s.into(),
            end: en.into(),
            project: "Alpha".into(),
            comment: String::new(),
        };
        let existing = vec![e(1, t(8, 0), t(12, 0))];
        // Straight on from noon: one seamless eight-hour session.
        let ok = validate_form(&fd(None, "1200", "1600"), &existing, &m.rules).unwrap();
        assert_eq!(ok.3, crate::core::Minutes(48));
        assert_eq!(ok.4, crate::core::Minutes(480 - 48));
        // Twenty minutes off already splits the day: 4:00 and 3:40, 18 each.
        let ok = validate_form(&fd(None, "1220", "1600"), &existing, &m.rules).unwrap();
        assert_eq!(ok.3, crate::core::Minutes(36));
        // Editing an entry previews the day as it would be, not as it is: the
        // old slot of the entry under edit must not count as its own neighbour.
        let existing = vec![e(1, t(8, 0), t(12, 0)), e(2, t(12, 45), t(17, 0))];
        // Pulling the afternoon entry back to 12:00 closes the only pause there is.
        let ok = validate_form(&fd(Some(2), "1200", "1700"), &existing, &m.rules).unwrap();
        assert_eq!(ok.3, crate::core::Minutes(48));
        assert_eq!(ok.4, crate::core::Minutes(540 - 48));
        // Leaving it where it is keeps two sessions of 4:00 and 4:15.
        let ok = validate_form(&fd(Some(2), "1245", "1700"), &existing, &m.rules).unwrap();
        assert_eq!(ok.3, crate::core::Minutes(36));
        assert_eq!(ok.4, crate::core::Minutes(495 - 36));
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
                    break_share: None,
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

    #[test]
    fn validate_settings_names_the_field_that_is_wrong() {
        let today = d(2026, 9, 15);
        let good = SettingsData {
            start: "today".into(),
            balance: "+12:30".into(),
            target: "8:00".into(),
            vacation: "28".into(),
        };
        assert_eq!(
            validate_settings(&good, today).unwrap(),
            ConfigPatch {
                start_date: Some(today),
                initial_balance_minutes: Some(750),
                daily_target_minutes: Some(480),
                vacation_days_per_year: Some(28),
                hours_format: None,
            }
        );
        let bad = |f: &dyn Fn(&mut SettingsData), needle: &str| {
            let mut d = good.clone();
            f(&mut d);
            let e = validate_settings(&d, today).unwrap_err();
            assert!(e.contains(needle), "{e}");
        };
        bad(&|d| d.start = "never".into(), "start");
        bad(&|d| d.balance = "abc".into(), "balance");
        bad(&|d| d.target = "zz".into(), "target");
        // Zero is a parseable duration but not a usable target.
        bad(&|d| d.target = "0:00".into(), "target");
        bad(&|d| d.vacation = "-1".into(), "vacation");
    }

    #[test]
    fn settings_footer_previews_the_patch() {
        let today = d(2026, 9, 15);
        let patch = validate_settings(
            &SettingsData {
                start: "today".into(),
                balance: "+12:30".into(),
                target: "8:00".into(),
                vacation: "28".into(),
            },
            today,
        )
        .unwrap();
        assert_eq!(
            settings_footer(&patch, HoursFormat::Hm),
            "balance +12:30 · target 08:00 · vacation 28"
        );
        assert_eq!(
            settings_footer(&patch, HoursFormat::Decimal),
            "balance +12.50h · target 8.00h · vacation 28"
        );
    }

    #[test]
    fn settings_overlay_opens_prefilled_and_writes_the_config() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        let dir = tempfile::tempdir().unwrap();
        m.home = dir.path().to_path_buf();

        // `c` on the month screen: prefilled from the running rules.
        assert_eq!(
            components::month::MonthScreen::default().on(&tuirealm::event::Event::Keyboard(
                tuirealm::event::KeyEvent::new(
                    tuirealm::event::Key::Char('c'),
                    tuirealm::event::KeyModifiers::NONE
                )
            )),
            Some(Msg::OpenSettings)
        );
        m.update(Msg::OpenSettings);
        let s = m.settings.as_ref().expect("the overlay is open");
        assert_eq!(s.data.start, today.to_string());
        assert_eq!(s.data.balance, "+00:00");
        assert_eq!(s.data.target, "07:48");
        assert_eq!(s.data.vacation, "30");
        assert_eq!(s.error, None, "a freshly opened overlay does not complain");

        m.update(Msg::SettingsSubmit(SettingsData {
            start: "2026-01-01".into(),
            balance: "-2:30".into(),
            target: "8:00".into(),
            vacation: "28".into(),
        }));
        let written = std::fs::read_to_string(dir.path().join("config.toml")).unwrap();
        assert!(written.contains("daily_target_minutes = 480"), "{written}");
        assert_eq!(m.rules.daily_target, Minutes(480));
        assert_eq!(m.rules.start_date, d(2026, 1, 1));
        assert_eq!(m.rules.initial_balance, Minutes(-150));
        assert_eq!(m.vacation_allowance, 28);
        // The month is reloaded, because every balance in it just moved.
        assert!(matches!(rx.try_recv().unwrap(), StoreCmd::LoadMonth { .. }));
        assert!(m.settings.is_none(), "saving closes the overlay");
        let (msg, is_error, _) = m.status.as_ref().expect("a status message");
        assert_eq!(msg, "Settings saved");
        assert!(!is_error);
    }

    #[test]
    fn settings_overlay_stays_open_on_an_invalid_value() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        let dir = tempfile::tempdir().unwrap();
        m.home = dir.path().to_path_buf();
        m.update(Msg::OpenSettings);
        let bad = SettingsData {
            target: "zz".into(),
            ..m.settings.as_ref().unwrap().data.clone()
        };
        m.update(Msg::SettingsChanged(bad.clone()));
        let err = m.settings.as_ref().unwrap().error.clone().unwrap();
        assert!(err.contains("target"), "{err}");
        m.update(Msg::SettingsSubmit(bad));
        assert!(m.settings.is_some(), "an invalid submit keeps it open");
        assert!(
            !dir.path().join("config.toml").exists(),
            "nothing was written"
        );
        assert!(rx.try_recv().is_err());
        // Esc closes it without writing anything.
        m.update(Msg::SettingsCancel);
        assert!(m.settings.is_none());
        assert!(!dir.path().join("config.toml").exists());
    }

    #[test]
    fn month_hints_and_help_mention_the_settings_key() {
        let (m, _rx) = model(d(2026, 9, 15));
        assert!(m.key_hints().contains(&("c", "settings")));
        assert!(m.help_keys().contains(&("c", "settings")));
    }
}
