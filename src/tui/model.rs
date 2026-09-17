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
    ChartMode, Confirm, DayData, FormData, MonthData, Msg, ProjectsData, RangeKind, SettingsData,
    StatsData, StoreCmd, StoreReply, UserEvent,
};
use super::theme::Theme;
use super::view::chrome;
use super::worker::Worker;
use crate::config::{Config, ConfigPatch};
use crate::core::{
    Entry, HolidayCalendar, HoursFormat, Minutes, Rules, check_overlap, check_range, deduction,
    entry_nets, parse_date, parse_time, session_deduction, session_members,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Month,
    Day,
    Stats,
    // --- projects screen ---
    Projects,
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
    /// What to do about the break split once the store has answered: the box
    /// needs the day the entry landed in, and that day is only in hand after
    /// the refresh the write triggers.
    pub split_followup: Option<SplitFollowup>,
    pub break_split: Option<BreakSplitState>,
    /// The far end of the date range the day-type keys work on, dropped by `V`.
    /// The range is anchor..cursor in whichever order the two fall.
    pub anchor: Option<NaiveDate>,
    // --- projects screen ---
    /// Every project and what has been worked on it; `None` while loading.
    pub projects: Option<ProjectsData>,
    pub projects_cursor: usize,
    /// What the one-field name box on that screen is for, while it is open.
    pub prompt: Option<PromptKind>,
}

/// What the one-field name box is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptKind {
    RenameProject { old: String },
    AddProject,
}

/// What a write that touched a session's break should do once the refreshed day
/// arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitFollowup {
    /// Open the break-split box on the session this entry belongs to.
    Open { date: NaiveDate, entry_id: i64 },
    /// Only say where the break went and which key changes it: a day-editor
    /// edit is not the moment to put a box in the user's way.
    Hint,
}

/// State of the open break-split box: the session it is about, and the fields
/// as typed.
pub struct BreakSplitState {
    pub date: NaiveDate,
    /// The entries of the session, in clock order; every vector below is
    /// aligned with it.
    pub session_entry_ids: Vec<i64>,
    pub deduction: Minutes,
    pub grosses: Vec<Minutes>,
    /// What each row is called, so an error can name the entry it is about.
    pub labels: Vec<String>,
    pub values: Vec<String>,
    pub error: Option<String>,
}

/// What the open clock-in overlay is about, and so what submitting it does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockAction {
    /// Nothing is running: open the first session.
    In,
    /// A session is running: book it and open the next one.
    Switch,
    /// A break is on: go back to work, on this project.
    Resume,
}

/// State of the open clock-in overlay.
pub struct ClockPickerState {
    pub action: ClockAction,
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

/// Read the break-split fields back as shares, or say why they are not usable.
///
/// The rules are the ones the core allocation cannot express for the user: a
/// share has to be a number of minutes, no larger than the entry it sits on,
/// and the assigned shares of a session may not come to more than that session
/// actually loses. Everything left unassigned falls to the default rule, so an
/// empty box is always valid.
pub fn validate_break_split(
    values: &[String],
    labels: &[String],
    grosses: &[Minutes],
    deduction: Minutes,
    f: HoursFormat,
) -> Result<Vec<Option<Minutes>>, String> {
    let named = |i: usize| -> String {
        labels
            .get(i)
            .map(|l| l.split("  ").next().unwrap_or(l).to_string())
            .unwrap_or_else(|| format!("entry {}", i + 1))
    };
    let mut shares = Vec::with_capacity(values.len());
    for (i, v) in values.iter().enumerate() {
        let share =
            components::break_form::parse_share(v).map_err(|e| format!("{}: {e}", named(i)))?;
        if let Some(share) = share
            && let Some(gross) = grosses.get(i)
            && share > *gross
        {
            return Err(format!(
                "{}: at most {} minutes — that is all it is long",
                named(i),
                gross.0
            ));
        }
        shares.push(share);
    }
    let assigned: Minutes = shares.iter().flatten().copied().sum();
    if assigned > deduction {
        return Err(format!(
            "assigned {} of {} — more than this session's break",
            assigned.fmt_unsigned(f),
            deduction.fmt_unsigned(f)
        ));
    }
    Ok(shares)
}

/// The footer of a break-split box that validates: how much of the deduction
/// has been handed out by hand. The rest falls to the default rule.
pub fn break_split_footer(
    shares: &[Option<Minutes>],
    deduction: Minutes,
    f: HoursFormat,
) -> String {
    let assigned: Minutes = shares.iter().flatten().copied().sum();
    format!(
        "assigned {} / {}",
        assigned.fmt_unsigned(f),
        deduction.fmt_unsigned(f)
    )
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
            // --- projects screen ---
            Screen::Projects => Id::Projects,
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

    /// The entries of `date` as the model last saw them: the day editor's own
    /// day when that is the one asked for, otherwise the month it is drawing.
    fn entries_on(&self, date: NaiveDate) -> Vec<Entry> {
        if let Some(d) = self.day.as_ref().filter(|d| d.day.date == date) {
            return d.day.entries.clone();
        }
        self.month
            .as_ref()
            .and_then(|m| m.days.iter().find(|d| d.date == date))
            .map(|d| d.entries.clone())
            .unwrap_or_default()
    }

    /// Open the break-split box on the session `entry_id` belongs to.
    ///
    /// Everything the box shows is read off the day: which entries share the
    /// session, what that session loses, what each entry pays now, and what the
    /// default rule would give it. A session with nothing to split says so in
    /// the status bar instead of opening an empty box.
    fn open_break_split(&mut self, date: NaiveDate, entry_id: i64) {
        let entries = self.entries_on(date);
        let intervals: Vec<(i32, i32)> = entries.iter().map(Entry::interval).collect();
        let Some(group) = session_members(&intervals)
            .into_iter()
            .find(|g| g.iter().any(|&i| entries[i].id == entry_id))
        else {
            self.set_status("That entry is not on screen any more", true);
            return;
        };
        let span = {
            let a = group.iter().map(|&i| intervals[i].0).min().unwrap_or(0);
            let b = group.iter().map(|&i| intervals[i].1).max().unwrap_or(0);
            b - a
        };
        let ded = deduction(Minutes(span), &self.rules.tiers);
        if ded.0 == 0 {
            self.set_status("This session is too short to lose a break", false);
            return;
        }
        if group.len() < 2 {
            self.set_status("One entry in this session: the whole break is on it", false);
            return;
        }
        let session: Vec<Entry> = group.iter().map(|&i| entries[i].clone()).collect();
        // What the default rule alone would give each entry, as the placeholder:
        // the box shows what it is about to do, not only what was typed.
        let unassigned: Vec<Entry> = session
            .iter()
            .cloned()
            .map(|e| Entry {
                break_share: None,
                ..e
            })
            .collect();
        let defaults: Vec<Minutes> = entry_nets(&unassigned, &self.rules.tiers)
            .iter()
            .zip(&unassigned)
            .map(|(net, e)| e.duration() - *net)
            .collect();
        let fields: Vec<components::break_form::SplitField> = session
            .iter()
            .enumerate()
            .map(|(k, e)| components::break_form::SplitField {
                entry_id: e.id,
                label: format!(
                    "{}–{}  {}  ({})",
                    e.start.format("%H:%M"),
                    e.end.format("%H:%M"),
                    e.project,
                    e.duration().fmt_signed(self.hours)
                ),
                value: e.break_share.map(|m| m.0.to_string()).unwrap_or_default(),
                placeholder: defaults[k].0.to_string(),
            })
            .collect();
        let _ = self.app.umount(&Id::BreakSplit);
        let _ = self.app.mount(
            Id::BreakSplit,
            Box::new(
                components::break_form::BreakSplitForm::new(
                    format!("Break split — {date}"),
                    &fields,
                )
                .with_theme(&self.theme),
            ),
            vec![],
        );
        self.break_split = Some(BreakSplitState {
            date,
            session_entry_ids: session.iter().map(|e| e.id).collect(),
            deduction: ded,
            grosses: session.iter().map(Entry::duration).collect(),
            labels: fields.iter().map(|f| f.label.clone()).collect(),
            values: fields.iter().map(|f| f.value.clone()).collect(),
            error: None,
        });
        // A box that only shows what is already stored has nothing to complain
        // about yet.
        self.refresh_break_split(false);
        self.focus(Id::BreakSplit);
    }

    fn close_break_split(&mut self) {
        self.break_split = None;
        let _ = self.app.umount(&Id::BreakSplit);
        self.focus_screen();
    }

    /// Re-validate the open break-split box and push its footer line back in.
    fn refresh_break_split(&mut self, show_error: bool) {
        let Some(st) = &self.break_split else { return };
        let (error, footer) = match validate_break_split(
            &st.values,
            &st.labels,
            &st.grosses,
            st.deduction,
            self.hours,
        ) {
            Ok(shares) => (
                None,
                Some(break_split_footer(&shares, st.deduction, self.hours)),
            ),
            Err(e) => (Some(e), None),
        };
        let error = if show_error { error } else { None };
        let (text, is_error) = match (&error, &footer) {
            (Some(e), _) => (e.clone(), true),
            (None, Some(f)) => (f.clone(), false),
            _ => (String::new(), false),
        };
        if let Some(st) = &mut self.break_split {
            st.error = error;
        }
        let _ = self
            .app
            .attr(&Id::BreakSplit, Attribute::Text, AttrValue::String(text));
        let _ = self.app.attr(
            &Id::BreakSplit,
            Attribute::Custom(components::form::ERROR_FLAG),
            AttrValue::Flag(is_error),
        );
    }

    /// Whether the session `entry_id` sits in spans more than one project and
    /// loses a break at all — the two conditions that make a split worth
    /// offering. `entries` is the whole day.
    fn split_is_worth_offering(&self, entries: &[Entry], entry_id: i64) -> bool {
        let intervals: Vec<(i32, i32)> = entries.iter().map(Entry::interval).collect();
        let Some(group) = session_members(&intervals)
            .into_iter()
            .find(|g| g.iter().any(|&i| entries[i].id == entry_id))
        else {
            return false;
        };
        let span = {
            let a = group.iter().map(|&i| intervals[i].0).min().unwrap_or(0);
            let b = group.iter().map(|&i| intervals[i].1).max().unwrap_or(0);
            b - a
        };
        if deduction(Minutes(span), &self.rules.tiers).0 == 0 {
            return false;
        }
        let mut projects: Vec<&str> = group.iter().map(|&i| entries[i].project.as_str()).collect();
        projects.sort_unstable();
        projects.dedup();
        projects.len() > 1
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

    /// The projects the resume overlay offers: the one the break is on first, so
    /// Enter comes back to it, and every other known project after it.
    fn resume_projects(&self, remembered: Option<&str>) -> Vec<String> {
        let mut names = self.picker_projects(None);
        if let Some(p) = remembered {
            names.retain(|n| n != p);
            names.insert(0, p.to_string());
        }
        names
    }

    /// Mount a fresh clock-in overlay over the month screen and give it focus.
    fn open_clock_picker(&mut self, title: String, projects: Vec<String>, action: ClockAction) {
        let _ = self.app.umount(&Id::ClockPicker);
        let _ = self.app.mount(
            Id::ClockPicker,
            Box::new(
                components::project_picker::ProjectPicker::new(title, projects)
                    .with_theme(&self.theme),
            ),
            vec![],
        );
        self.clock_picker = Some(ClockPickerState { action });
        self.focus(Id::ClockPicker);
    }

    fn close_clock_picker(&mut self) {
        self.clock_picker = None;
        let _ = self.app.umount(&Id::ClockPicker);
        self.focus_screen();
    }

    // --- projects screen ---

    /// The project the cursor is on, if the screen has its data.
    fn highlighted_project(&self) -> Option<&crate::core::Project> {
        self.projects.as_ref()?.projects.get(self.projects_cursor)
    }

    /// Mount a fresh name box over the projects screen and give it focus.
    fn open_prompt(&mut self, title: String, initial: &str, kind: PromptKind) {
        let _ = self.app.umount(&Id::Prompt);
        let _ = self.app.mount(
            Id::Prompt,
            Box::new(
                components::text_prompt::TextPrompt::new(title, "Name", initial)
                    .with_theme(&self.theme),
            ),
            vec![],
        );
        self.prompt = Some(kind);
        self.focus(Id::Prompt);
    }

    fn close_prompt(&mut self) {
        self.prompt = None;
        let _ = self.app.umount(&Id::Prompt);
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
                } else if self.screen == Screen::Month && self.anchor.is_some() {
                    self.anchor = None;
                    self.set_status("Range cleared", false);
                } else {
                    // The day editor writes entries and the projects screen
                    // renames projects: either way the month on screen is stale.
                    let reload = matches!(self.screen, Screen::Day | Screen::Projects);
                    self.screen = Screen::Month;
                    let _ = self.app.active(&Id::Month);
                    if reload {
                        self.load_month();
                    }
                }
            }
            Msg::OpenClockPicker => {
                // No weekday condition: weekend work counts towards the balance, and
                // the CLI (`tk in`) has always allowed it.
                let session = self.month.as_ref().and_then(|m| m.session.clone());
                let current = session.as_ref().and_then(|s| s.project.clone());
                let action = match &session {
                    Some(s) if s.state.is_break() => ClockAction::Resume,
                    Some(_) => ClockAction::Switch,
                    None => ClockAction::In,
                };
                let title = match (action, current.as_deref()) {
                    // Coming back from a break is not a switch: the project on
                    // the clock is the one being resumed, so it is offered, not
                    // left out.
                    (ClockAction::Resume, Some(p)) => format!("Resume — currently {p}"),
                    (ClockAction::Resume, None) => "Resume — project".to_string(),
                    (ClockAction::Switch, Some(p)) => format!("Switch project — currently {p}"),
                    (ClockAction::Switch, None) => "Switch project".to_string(),
                    (ClockAction::In, _) => "Clock in — project".to_string(),
                };
                let projects = match action {
                    ClockAction::Resume => self.resume_projects(current.as_deref()),
                    _ => self.picker_projects(current.as_deref()),
                };
                self.open_clock_picker(title, projects, action);
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
                match state.action {
                    ClockAction::Switch => self.send(StoreCmd::Switch { project: name }),
                    ClockAction::Resume => self.send(StoreCmd::Resume {
                        project: Some(name),
                    }),
                    ClockAction::In => self.send(StoreCmd::ClockIn(
                        self.today,
                        self.now.with_second(0).unwrap(),
                        name,
                    )),
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
            Msg::TakeBreak => {
                let session = self.month.as_ref().and_then(|m| m.session.as_ref());
                match session {
                    None => self.set_status("Not clocked in", true),
                    Some(s) if s.state.is_break() => self.set_status("Already on break", true),
                    Some(_) => self.send(StoreCmd::Break),
                }
            }
            Msg::SetKind(kind) => {
                // A range is the worker's to check: one day of it having entries
                // or falling on a weekend is no reason to refuse the whole thing
                // here, where only the loaded month is in hand.
                if let Some(a) = self.anchor {
                    let (from, to) = (a.min(self.selected), a.max(self.selected));
                    self.open_confirm(Confirm::SetKindRange { from, to, kind });
                    return;
                }
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
                    // --- range marking ---
                    Confirm::SetKindRange { from, to, kind } => {
                        self.anchor = None;
                        self.send(StoreCmd::SetKindRange { from, to, kind });
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
            // --- break split ---
            Msg::OpenBreakSplit { date, entry_id } => self.open_break_split(date, entry_id),
            Msg::DayBreakSplit => {
                let Some(d) = &self.day else { return };
                let Some(e) = d.day.entries.get(self.day_cursor) else {
                    self.set_status("No entry to split", true);
                    return;
                };
                let (date, id) = (d.day.date, e.id);
                self.open_break_split(date, id);
            }
            Msg::BreakSplitChanged(values) => {
                if let Some(st) = &mut self.break_split {
                    st.values = values;
                }
                self.refresh_break_split(true);
            }
            Msg::BreakSplitCancel => self.close_break_split(),
            Msg::BreakSplitSubmit(shares) => {
                let Some(st) = &mut self.break_split else {
                    return;
                };
                // Read the submitted shares back as the fields they came from,
                // so one validator covers both a keystroke and a save.
                st.values = shares
                    .iter()
                    .map(|(_, m)| m.map(|m| m.0.to_string()).unwrap_or_default())
                    .collect();
                let st = self.break_split.as_ref().expect("just borrowed");
                match validate_break_split(
                    &st.values,
                    &st.labels,
                    &st.grosses,
                    st.deduction,
                    self.hours,
                ) {
                    Err(_) => self.refresh_break_split(true),
                    Ok(_) => {
                        self.send(StoreCmd::SetBreakShares(shares));
                        self.close_break_split();
                    }
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
                        // The day as saving this form will leave it: if that puts
                        // a break on a session of several projects, say so once
                        // the store has written it.
                        let this = Entry {
                            id: data.id.unwrap_or(i64::MIN),
                            date: day.day.date,
                            start,
                            end,
                            project: project.clone(),
                            comment: comment.clone(),
                            break_share: None,
                        };
                        let saved: Vec<Entry> = day
                            .day
                            .entries
                            .iter()
                            .filter(|e| Some(e.id) != data.id)
                            .cloned()
                            .chain(std::iter::once(this.clone()))
                            .collect();
                        self.split_followup = self
                            .split_is_worth_offering(&saved, this.id)
                            .then_some(SplitFollowup::Hint);
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
            // --- backup key ---
            Msg::Backup => self.send(StoreCmd::Backup),
            // --- range marking ---
            Msg::ToggleAnchor => {
                if self.anchor.take().is_some() {
                    self.set_status("Range cleared", false);
                } else {
                    self.anchor = Some(self.selected);
                    self.set_status(
                        format!(
                            "Range from {}: v f x p w mark every weekday up to the cursor, V or Esc clears",
                            self.selected.format("%a %d %b")
                        ),
                        false,
                    );
                }
            }
            // --- projects screen ---
            Msg::OpenProjects => {
                self.screen = Screen::Projects;
                self.projects = None;
                self.projects_cursor = 0;
                self.focus(Id::Projects);
                self.send(StoreCmd::LoadProjects);
            }
            Msg::ProjectsSelect(n) => {
                let len = self.projects.as_ref().map_or(0, |p| p.projects.len());
                if len > 0 {
                    self.projects_cursor =
                        (self.projects_cursor as i32 + n).clamp(0, len as i32 - 1) as usize;
                }
            }
            Msg::ProjectsToggleArchive => {
                if let Some(p) = self.highlighted_project().cloned() {
                    self.send(StoreCmd::ArchiveProject {
                        name: p.name,
                        archived: !p.archived,
                    });
                }
            }
            Msg::ProjectsRename => {
                if let Some(p) = self.highlighted_project().cloned() {
                    self.open_prompt(
                        "Rename project".into(),
                        &p.name,
                        PromptKind::RenameProject {
                            old: p.name.clone(),
                        },
                    );
                }
            }
            Msg::ProjectsAdd => self.open_prompt("New project".into(), "", PromptKind::AddProject),
            Msg::PromptChanged => {}
            Msg::PromptSubmit(text) => {
                let name = text.trim().to_string();
                if name.is_empty() {
                    self.set_status("Name must not be empty", true);
                    return;
                }
                let Some(kind) = self.prompt.clone() else {
                    return;
                };
                self.close_prompt();
                match kind {
                    // Renaming a project to what it is already called is no
                    // rename at all: the box just closes.
                    PromptKind::RenameProject { old } if old == name => {}
                    PromptKind::RenameProject { old } => {
                        self.send(StoreCmd::RenameProject { old, new: name })
                    }
                    PromptKind::AddProject => self.send(StoreCmd::AddProject(name)),
                }
            }
            Msg::PromptCancel => self.close_prompt(),
        }
    }

    fn on_store(&mut self, reply: StoreReply) {
        match reply {
            StoreReply::Month(m) => {
                // A split still waiting for its day is dropped when the month
                // that comes back cannot hold that day at all — an overnight
                // session booked on yesterday's date while October is on
                // screen, say. Without this it would stay armed and spring the
                // box open on some unrelated refresh later. The day editor's
                // own date is spared: its reload is still on its way.
                if let Some(SplitFollowup::Open { date, .. }) = self.split_followup
                    && self.selected != date
                    && !m.days.iter().any(|d| d.date == date)
                {
                    self.split_followup = None;
                }
                self.month = Some(m)
            }
            StoreReply::Day(d) => {
                self.day_cursor = self.day_cursor.min(d.day.entries.len().saturating_sub(1));
                self.day = Some(d);
            }
            StoreReply::Stats(s) => self.stats = Some(s),
            StoreReply::Changed(msg) => {
                if !msg.is_empty() {
                    self.set_status(msg, false);
                }
                if self.split_followup == Some(SplitFollowup::Hint) {
                    self.split_followup = None;
                    self.set_status("break on last project · press b to split", false);
                }
                self.reload_after_write();
            }
            StoreReply::Booked {
                entry_id,
                date,
                session_projects,
                deduction,
                message,
            } => {
                self.set_status(message, false);
                // Worth asking about only when there is a break to split and
                // more than one project it could fall on. The box itself needs
                // the day the entry landed in, which the refresh brings.
                if session_projects > 1 && deduction.0 > 0 {
                    self.split_followup = Some(SplitFollowup::Open { date, entry_id });
                }
                self.reload_after_write();
            }
            StoreReply::Failed(e) => self.set_status(e, true),
            // --- projects screen ---
            StoreReply::Projects(d) => {
                // A project that was just renamed or archived keeps the cursor
                // where it was, and a shorter list pulls it back onto the last row.
                self.projects_cursor = self.projects_cursor.min(d.projects.len().saturating_sub(1));
                self.projects = Some(d);
            }
        }
        // A box waiting for its day opens as soon as that day is in hand — and
        // not before: the reply that asked for it arrives while the day on
        // screen is still the one from before the write, which may well hold
        // other entries but not yet the one just booked.
        if let Some(SplitFollowup::Open { date, entry_id }) = self.split_followup
            && self.entries_on(date).iter().any(|e| e.id == entry_id)
        {
            self.split_followup = None;
            self.open_break_split(date, entry_id);
        }
    }

    /// Reload what a write just changed: the month always, the day editor's own
    /// day when it is the screen being looked at.
    fn reload_after_write(&mut self) {
        self.load_month();
        if self.screen == Screen::Day {
            self.send(StoreCmd::LoadDay(self.selected));
        }
        if self.screen == Screen::Projects {
            self.send(StoreCmd::LoadProjects);
        }
    }

    pub fn view(&mut self) {
        let mut term = self.terminal.take().expect("terminal");
        let form_open = self.form.is_some();
        let settings_open = self.settings.is_some();
        let picker_open = self.clock_picker.is_some();
        let split_open = self.break_split.is_some();
        let prompt_open = self.prompt.is_some();
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
            if split_open {
                self.app.view(&Id::BreakSplit, f, area);
            }
            if prompt_open {
                self.app.view(&Id::Prompt, f, area);
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
            // --- projects screen ---
            Screen::Projects => super::view::projects::draw(self, f, body),
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
                // instead of freezing at 00:00 when the clock passes midnight. On a
                // break the same two figures are the pause and when it started.
                chrome::ClockInfo {
                    project: s.project.clone().unwrap_or_default(),
                    since: s.start.format("%H:%M").to_string(),
                    running: crate::core::running_minutes(
                        NaiveDateTime::new(s.date, s.start),
                        NaiveDateTime::new(self.today, self.now),
                    ),
                    on_break: s.state.is_break(),
                }
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
                // "break split" would take the row to 81 columns, one past the
                // 80 the UI promises; the `?` help spells the key out in full.
                ("b", "break"),
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
            // --- projects screen ---
            Screen::Projects => &[
                ("↑↓", "project"),
                ("a", "archive"),
                ("r", "rename"),
                ("n", "new"),
                ("u", "units"),
                ("?", "help"),
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
                ("P", "projects: archive, rename, add"),
                ("i", "clock in / switch project / resume"),
                ("o", "clock out"),
                ("b", "take a break (books work so far)"),
                ("v", "vacation"),
                ("f", "flex day"),
                ("x", "sick"),
                ("p", "public holiday"),
                ("w", "reset to work day"),
                ("V", "start / clear a range for the day-type keys"),
                ("u", "toggle h:mm / decimal hours"),
                ("B", "back up the database"),
                ("q", "quit"),
            ],
            Screen::Day => &[
                ("↑ ↓", "select entry"),
                ("a", "add entry"),
                ("e", "edit entry"),
                ("d", "delete entry"),
                ("b", "split this session's break"),
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
            // --- projects screen ---
            Screen::Projects => &[
                ("↑ ↓ j k", "select project"),
                ("a", "archive / unarchive"),
                ("r", "rename"),
                ("n", "new project"),
                ("u", "toggle h:mm / decimal hours"),
                ("Esc", "back"),
            ],
        }
    }

    pub fn confirm_text(&self, c: &Confirm) -> String {
        match c {
            Confirm::DeleteEntry(_) => "Delete this entry?".into(),
            Confirm::SetKind(d, k) => format!("Set {d} to {}?", k.display_name().to_lowercase()),
            // --- range marking ---
            Confirm::SetKindRange { from, to, kind } => {
                let n = from
                    .iter_days()
                    .take_while(|d| d <= to)
                    .filter(|d| crate::core::is_working_day(*d))
                    .count();
                format!(
                    "Set {n} weekdays, {from} to {to}, to {}?",
                    kind.display_name().to_lowercase()
                )
            }
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
            split_followup: None,
            break_split: None,
            anchor: None,
            projects: None,
            projects_cursor: 0,
            prompt: None,
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
        assert_eq!(st.action, ClockAction::In, "nothing is running yet");
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
        assert_eq!(
            m.clock_picker.as_ref().expect("open").action,
            ClockAction::Switch
        );
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

    /// A session on a break: what the month view has in hand after a `b`.
    fn paused_session(today: NaiveDate, project: &str) -> crate::store::Session {
        crate::store::Session {
            date: today,
            start: NaiveTime::from_hms_opt(12, 3, 0).unwrap(),
            project: Some(project.to_string()),
            state: crate::store::SessionState::Break,
        }
    }

    #[test]
    fn b_takes_a_break_only_while_the_clock_runs() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        // Nothing running: nothing to interrupt.
        m.month = Some(month_with_projects(today, &["Alpha"], Some("Alpha"), None));
        m.update(Msg::TakeBreak);
        assert!(m.status.as_ref().unwrap().1, "shown as an error");
        assert!(rx.try_recv().is_err());

        // Clocked in and working: `b` books the work so far.
        let sess = crate::store::Session {
            date: today,
            start: NaiveTime::from_hms_opt(8, 12, 0).unwrap(),
            project: Some("Alpha".into()),
            state: crate::store::SessionState::Working,
        };
        m.month = Some(month_with_projects(
            today,
            &["Alpha"],
            Some("Alpha"),
            Some(sess),
        ));
        m.update(Msg::TakeBreak);
        assert!(matches!(rx.try_recv().unwrap(), StoreCmd::Break));

        // Already on a break: there is nothing left to book.
        m.month = Some(month_with_projects(
            today,
            &["Alpha"],
            Some("Alpha"),
            Some(paused_session(today, "Alpha")),
        ));
        m.update(Msg::TakeBreak);
        assert!(m.status.as_ref().unwrap().0.contains("Already on break"));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn on_a_break_i_resumes_and_o_ends_it() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        m.month = Some(month_with_projects(
            today,
            &["Alpha", "Beta"],
            Some("Alpha"),
            Some(paused_session(today, "Alpha")),
        ));
        m.update(Msg::OpenClockPicker);
        assert_eq!(
            m.clock_picker.as_ref().expect("open").action,
            ClockAction::Resume
        );
        // The project the break is on is offered first, so Enter comes back to
        // it — unlike a switch, which leaves the running project out.
        assert_eq!(
            m.resume_projects(Some("Alpha")),
            vec!["Alpha".to_string(), "Beta".into()]
        );
        m.update(Msg::ClockPickerSubmit("Alpha".into()));
        assert!(matches!(
            rx.try_recv().unwrap(),
            StoreCmd::Resume { project: Some(p) } if p == "Alpha"
        ));
        // Another project is a resume too, not a switch.
        m.update(Msg::OpenClockPicker);
        m.update(Msg::ClockPickerSubmit("Beta".into()));
        assert!(matches!(
            rx.try_recv().unwrap(),
            StoreCmd::Resume { project: Some(p) } if p == "Beta"
        ));
        // `o` ends the break: the store knows there is nothing to book.
        m.update(Msg::ClockOut);
        assert!(matches!(
            rx.try_recv().unwrap(),
            StoreCmd::ClockOut { project: None, .. }
        ));
    }

    #[test]
    fn the_title_bar_says_when_a_break_is_on() {
        let today = d(2026, 9, 15);
        let (mut m, _rx) = model(today);
        m.now = NaiveTime::from_hms_opt(12, 15, 0).unwrap();
        m.month = Some(month_data(
            today,
            vec![],
            Some(paused_session(today, "Alpha")),
        ));
        let info = m.title_info();
        assert_eq!(
            info.clock,
            Some(chrome::ClockInfo {
                project: "Alpha".into(),
                since: "12:03".into(),
                running: Minutes(12),
                on_break: true,
            })
        );
        // The help names the key, even where the hint row has no room for it.
        assert!(
            m.help_keys()
                .iter()
                .any(|(k, d)| *k == "b" && d.contains("break")),
            "{:?}",
            m.help_keys()
        );
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
            Some(chrome::ClockInfo {
                project: "Alpha".to_string(),
                since: "23:00".to_string(),
                running: crate::core::Minutes(120),
                on_break: false,
            })
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

    /// A day worked straight through on two projects, as the day editor and the
    /// month both hold it: one nine-hour session losing 48 minutes.
    fn two_project_day(today: NaiveDate) -> Vec<Entry> {
        let t = |h| NaiveTime::from_hms_opt(h, 0, 0).unwrap();
        vec![
            Entry {
                id: 1,
                date: today,
                start: t(8),
                end: t(12),
                project: "Alpha".into(),
                comment: "morning".into(),
                break_share: None,
            },
            Entry {
                id: 2,
                date: today,
                start: t(12),
                end: t(17),
                project: "Beta".into(),
                comment: "afternoon".into(),
                break_share: None,
            },
        ]
    }

    fn day_screen_with(m: &mut Model, today: NaiveDate, entries: Vec<Entry>) {
        m.screen = Screen::Day;
        m.day = Some(crate::tui::msg::DayData {
            day: Day {
                date: today,
                kind: DayKind::Work,
                entries,
            },
            projects: vec![],
        });
    }

    #[test]
    fn validate_break_split_checks_the_minutes_the_caps_and_the_total() {
        let labels = vec![
            "08:00–12:00  Alpha  (+04:00)".to_string(),
            "12:00–17:00  Beta  (+05:00)".to_string(),
        ];
        let grosses = vec![Minutes(240), Minutes(300)];
        let ded = Minutes(48);
        let v = |a: &str, b: &str| vec![a.to_string(), b.to_string()];
        let ok = |vals: Vec<String>| {
            validate_break_split(&vals, &labels, &grosses, ded, HoursFormat::Hm).unwrap()
        };
        // An empty box is the default rule, and is always valid.
        assert_eq!(ok(v("", "")), vec![None, None]);
        assert_eq!(ok(v("30", "")), vec![Some(Minutes(30)), None]);
        assert_eq!(
            ok(v("30", "18")),
            vec![Some(Minutes(30)), Some(Minutes(18))]
        );
        // A zero is a share, not an absence of one.
        assert_eq!(ok(v("0", "")), vec![Some(Minutes::ZERO), None]);
        let err = |vals: Vec<String>| {
            validate_break_split(&vals, &labels, &grosses, ded, HoursFormat::Hm).unwrap_err()
        };
        // Not minutes at all, and the error names the entry it is about.
        let e = err(v("", "half an hour"));
        assert!(e.contains("12:00–17:00"), "{e}");
        assert!(e.contains("not a number of minutes"), "{e}");
        assert!(err(v("-5", "")).contains("08:00–12:00"));
        assert!(err(v("0:30", "")).contains("08:00–12:00"));
        // More than the entry is long, and more than the session loses.
        let e = err(v("300", ""));
        assert!(e.contains("at most 240"), "{e}");
        let e = err(v("30", "30"));
        assert!(e.contains("assigned 01:00 of 00:48"), "{e}");
        assert!(e.contains("more than this session's break"), "{e}");
        // The footer says how much has been handed out by hand.
        assert_eq!(
            break_split_footer(&ok(v("30", "")), ded, HoursFormat::Hm),
            "assigned 00:30 / 00:48"
        );
        assert_eq!(
            break_split_footer(&ok(v("", "")), ded, HoursFormat::Hm),
            "assigned 00:00 / 00:48"
        );
    }

    #[test]
    fn b_opens_the_break_split_on_the_session_of_the_selected_entry() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        day_screen_with(&mut m, today, two_project_day(today));
        // The day editor's `b` is what asks for it.
        assert_eq!(
            components::day::DayScreen::default().on(&tuirealm::event::Event::Keyboard(
                tuirealm::event::KeyEvent::new(
                    tuirealm::event::Key::Char('b'),
                    tuirealm::event::KeyModifiers::NONE
                )
            )),
            Some(Msg::DayBreakSplit)
        );
        m.update(Msg::DayBreakSplit);
        let st = m.break_split.as_ref().expect("the box is open");
        // Both entries of the session, in clock order, whichever one the cursor
        // was on, and the session's own deduction.
        assert_eq!(st.session_entry_ids, vec![1, 2]);
        assert_eq!(st.deduction, Minutes(48));
        assert_eq!(st.grosses, vec![Minutes(240), Minutes(300)]);
        assert_eq!(st.values, vec![String::new(), String::new()]);
        assert_eq!(st.labels[0], "08:00–12:00  Alpha  (+04:00)");
        assert_eq!(st.error, None, "a box that only shows the stored shares");
        assert!(m.app.mounted(&Id::BreakSplit));
        // Saving is the store's business, and it names the entries.
        m.update(Msg::BreakSplitSubmit(vec![
            (1, Some(Minutes(30))),
            (2, None),
        ]));
        assert!(m.break_split.is_none(), "saving closes the box");
        assert!(!m.app.mounted(&Id::BreakSplit));
        match rx.try_recv().unwrap() {
            StoreCmd::SetBreakShares(shares) => {
                assert_eq!(shares, vec![(1, Some(Minutes(30))), (2, None)]);
            }
            other => panic!("expected SetBreakShares, got {other:?}"),
        }

        // A share already stored is what the box opens on.
        let mut entries = two_project_day(today);
        entries[0].break_share = Some(Minutes(30));
        day_screen_with(&mut m, today, entries);
        m.update(Msg::DayBreakSplit);
        assert_eq!(
            m.break_split.as_ref().unwrap().values,
            vec!["30".to_string(), String::new()]
        );
        // Esc closes it without sending anything.
        m.update(Msg::BreakSplitCancel);
        assert!(m.break_split.is_none());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_session_with_nothing_to_split_says_so_instead_of_opening() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        let t = |h| NaiveTime::from_hms_opt(h, 0, 0).unwrap();
        // One entry is one session: the whole break is on it, so there is
        // nothing to share out.
        day_screen_with(
            &mut m,
            today,
            vec![Entry {
                id: 1,
                date: today,
                start: t(8),
                end: t(17),
                project: "Alpha".into(),
                comment: String::new(),
                break_share: None,
            }],
        );
        m.update(Msg::DayBreakSplit);
        assert!(m.break_split.is_none());
        assert!(
            m.status
                .as_ref()
                .unwrap()
                .0
                .contains("whole break is on it"),
            "{:?}",
            m.status
        );
        // A session under the first tier loses nothing to split.
        day_screen_with(
            &mut m,
            today,
            vec![
                Entry {
                    id: 1,
                    date: today,
                    start: t(8),
                    end: t(9),
                    project: "Alpha".into(),
                    comment: String::new(),
                    break_share: None,
                },
                Entry {
                    id: 2,
                    date: today,
                    start: t(9),
                    end: t(10),
                    project: "Beta".into(),
                    comment: String::new(),
                    break_share: None,
                },
            ],
        );
        m.update(Msg::DayBreakSplit);
        assert!(m.break_split.is_none());
        assert!(
            m.status.as_ref().unwrap().0.contains("too short"),
            "{:?}",
            m.status
        );
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn over_assigning_the_break_is_refused_in_the_footer() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        day_screen_with(&mut m, today, two_project_day(today));
        m.update(Msg::DayBreakSplit);
        m.update(Msg::BreakSplitChanged(vec![
            "60".to_string(),
            "60".to_string(),
        ]));
        let err = m.break_split.as_ref().unwrap().error.clone().unwrap();
        assert!(err.contains("more than this session's break"), "{err}");
        // And a submit of the same figures is refused too: the box stays open
        // and nothing reaches the store.
        m.update(Msg::BreakSplitSubmit(vec![
            (1, Some(Minutes(60))),
            (2, Some(Minutes(60))),
        ]));
        assert!(m.break_split.is_some(), "an invalid save keeps it open");
        assert!(rx.try_recv().is_err());
        // Bringing it back inside the deduction saves.
        m.update(Msg::BreakSplitSubmit(vec![
            (1, Some(Minutes(24))),
            (2, Some(Minutes(24))),
        ]));
        assert!(m.break_split.is_none());
        assert!(matches!(
            rx.try_recv().unwrap(),
            StoreCmd::SetBreakShares(_)
        ));
    }

    /// The reply a clock-out sends back carries what the box needs to decide
    /// whether to open itself: a break to split, and more than one project it
    /// could fall on.
    #[test]
    fn a_multi_project_clock_out_opens_the_box_by_itself() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        m.month = Some(month_data(
            today,
            vec![Day {
                date: today,
                kind: DayKind::Work,
                entries: two_project_day(today),
            }],
            None,
        ));
        m.update(Msg::Store(StoreReply::Booked {
            entry_id: 2,
            date: today,
            session_projects: 2,
            deduction: Minutes(48),
            message: "Clocked out: 12:00–17:00 Beta (+05:00)".into(),
        }));
        let st = m.break_split.as_ref().expect("the box opened by itself");
        assert_eq!(st.session_entry_ids, vec![1, 2]);
        assert_eq!(st.deduction, Minutes(48));
        // The clock-out still says what it booked.
        assert_eq!(
            m.status.as_ref().unwrap().0,
            "Clocked out: 12:00–17:00 Beta (+05:00)"
        );
        // …and the month was reloaded, as after any write.
        assert!(
            rx.try_iter()
                .any(|c| matches!(c, StoreCmd::LoadMonth { .. }))
        );
        m.update(Msg::BreakSplitCancel);

        // One project in the session: nothing to ask about.
        m.update(Msg::Store(StoreReply::Booked {
            entry_id: 2,
            date: today,
            session_projects: 1,
            deduction: Minutes(48),
            message: "Clocked out: 12:00–17:00 Beta (+05:00)".into(),
        }));
        assert!(m.break_split.is_none());
        assert_eq!(m.split_followup, None);
        // Nor when the session loses nothing.
        m.update(Msg::Store(StoreReply::Booked {
            entry_id: 2,
            date: today,
            session_projects: 2,
            deduction: Minutes::ZERO,
            message: "Clocked out: 12:00–17:00 Beta (+05:00)".into(),
        }));
        assert!(m.break_split.is_none());
    }

    /// The reply comes back before the refreshed day does, and the day on screen
    /// is the one from before the clock-out — the morning it booked earlier, but
    /// not the entry just written. The box has to wait for the entry it is about.
    #[test]
    fn the_box_waits_for_the_day_that_holds_the_booked_entry() {
        let today = d(2026, 9, 15);
        let (mut m, _rx) = model(today);
        let entries = two_project_day(today);
        // Only the morning is on screen so far.
        m.month = Some(month_data(
            today,
            vec![Day {
                date: today,
                kind: DayKind::Work,
                entries: vec![entries[0].clone()],
            }],
            None,
        ));
        m.update(Msg::Store(StoreReply::Booked {
            entry_id: 2,
            date: today,
            session_projects: 2,
            deduction: Minutes(48),
            message: "Clocked out: 12:00–17:00 Beta (+05:00)".into(),
        }));
        assert!(m.break_split.is_none(), "the entry is not on screen yet");
        assert_eq!(
            m.split_followup,
            Some(SplitFollowup::Open {
                date: today,
                entry_id: 2
            }),
            "so the box stays queued"
        );
        // The refresh brings the entry, and the box opens on its session.
        m.update(Msg::Store(StoreReply::Month(month_data(
            today,
            vec![Day {
                date: today,
                kind: DayKind::Work,
                entries,
            }],
            None,
        ))));
        assert_eq!(
            m.break_split
                .as_ref()
                .expect("now it opens")
                .session_entry_ids,
            vec![1, 2]
        );
        assert_eq!(m.split_followup, None);
    }

    /// A queued split whose day the month on screen cannot hold is dropped: an
    /// overnight session booked on yesterday's date while another month is up
    /// must not leave the box armed to spring open on a later refresh.
    #[test]
    fn a_queued_split_expires_when_its_day_is_not_in_the_month() {
        let today = d(2026, 9, 15);
        let (mut m, _rx) = model(today);
        // The user has walked to October; the clock-out books into September.
        m.selected = d(2026, 10, 5);
        m.month = Some(month_data(today, vec![], None));
        m.update(Msg::Store(StoreReply::Booked {
            entry_id: 2,
            date: today,
            session_projects: 2,
            deduction: Minutes(48),
            message: "Clocked out: 12:00–17:00 Beta (+05:00)".into(),
        }));
        assert_eq!(
            m.split_followup,
            Some(SplitFollowup::Open {
                date: today,
                entry_id: 2
            }),
            "queued: the day is not in hand yet"
        );
        // October comes back, and it holds no September day.
        let october = MonthData {
            year: 2026,
            month: 10,
            days: vec![Day {
                date: d(2026, 10, 5),
                kind: DayKind::Work,
                entries: vec![],
            }],
            ..month_data(today, vec![], None)
        };
        m.update(Msg::Store(StoreReply::Month(october)));
        assert_eq!(m.split_followup, None, "the queued split is dropped");
        assert!(m.break_split.is_none());
        // And a later refresh of a month that does hold entries for that day
        // must not open anything: nothing is waiting any more.
        m.update(Msg::Store(StoreReply::Month(month_data(
            today,
            vec![Day {
                date: today,
                kind: DayKind::Work,
                entries: two_project_day(today),
            }],
            None,
        ))));
        assert!(m.break_split.is_none());
    }

    /// Saving an entry in the day editor is not the moment for a box, but the
    /// status bar says where the break went and which key moves it.
    #[test]
    fn saving_an_entry_hints_at_the_split() {
        let today = d(2026, 9, 15);
        let (mut m, _rx) = model(today);
        let entries = two_project_day(today);
        day_screen_with(&mut m, today, vec![entries[0].clone()]);
        // A second project straight on from noon: the session now spans two.
        m.update(Msg::FormSubmit(FormData {
            id: None,
            start: "1200".into(),
            end: "1700".into(),
            project: "Beta".into(),
            comment: String::new(),
        }));
        assert_eq!(m.split_followup, Some(SplitFollowup::Hint));
        m.update(Msg::Store(StoreReply::Changed(
            "Added 12:00–17:00 Beta".into(),
        )));
        assert_eq!(
            m.status.as_ref().unwrap().0,
            "break on last project · press b to split"
        );
        assert_eq!(m.split_followup, None, "the hint is shown once");
        assert!(m.break_split.is_none(), "no box in the way");

        // An entry whose session stays on one project says nothing.
        day_screen_with(&mut m, today, vec![]);
        m.update(Msg::FormSubmit(FormData {
            id: None,
            start: "0800".into(),
            end: "1200".into(),
            project: "Alpha".into(),
            comment: String::new(),
        }));
        assert_eq!(m.split_followup, None);
    }

    /// The box over the day screen at 100×30: every row of the session, its
    /// share, and the footer that says how much has been assigned.
    #[test]
    fn the_break_split_box_renders_over_the_day_screen() {
        use tuirealm::ratatui::Terminal;
        use tuirealm::ratatui::backend::TestBackend;

        let today = d(2026, 9, 15);
        let (mut m, _rx) = model(today);
        day_screen_with(&mut m, today, two_project_day(today));
        m.update(Msg::DayBreakSplit);
        let mut term = Terminal::new(TestBackend::new(100, 30)).unwrap();
        term.draw(|f| {
            m.draw(f);
            let area = f.area();
            m.app.view(&Id::BreakSplit, f, area);
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        let rows: Vec<String> = (0..30)
            .map(|y| {
                (0..100)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect();
        let joined = rows.join("\n");
        assert!(
            rows.iter().any(|r| r.contains("Break split — 2026-09-15")),
            "{joined}"
        );
        assert!(
            rows.iter()
                .any(|r| r.contains("08:00–12:00  Alpha  (+04:00)")),
            "{joined}"
        );
        assert!(
            rows.iter()
                .any(|r| r.contains("12:00–17:00  Beta  (+05:00)")),
            "{joined}"
        );
        assert!(
            rows.iter().any(|r| r.contains("assigned 00:00 / 00:48")),
            "the footer:\n{joined}"
        );
        assert!(
            rows.iter().all(|r| r.chars().count() <= 100),
            "a row overflows:\n{joined}"
        );
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

    #[test]
    fn backup_key_sends_the_backup_command() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
        let (mut m, rx) = model(today);
        m.update(Msg::Backup);
        assert!(matches!(rx.try_recv().unwrap(), StoreCmd::Backup));
    }
    // --- range marking ---

    fn work_days(from: NaiveDate, to: NaiveDate) -> Vec<Day> {
        from.iter_days()
            .take_while(|d| *d <= to)
            .map(|date| Day {
                date,
                kind: DayKind::Work,
                entries: vec![],
            })
            .collect()
    }

    #[test]
    fn v_drops_and_clears_the_anchor() {
        let today = d(2026, 9, 15);
        let (mut m, _rx) = model(today);
        m.month = Some(month_data(
            today,
            work_days(d(2026, 9, 1), d(2026, 9, 30)),
            None,
        ));
        m.update(Msg::ToggleAnchor);
        assert_eq!(m.anchor, Some(today));
        assert_eq!(
            m.status.as_ref().unwrap().0,
            "Range from Tue 15 Sep: v f x p w mark every weekday up to the cursor, V or Esc clears"
        );
        m.update(Msg::SelectDay(3));
        assert_eq!(m.anchor, Some(today), "moving keeps the anchor");
        m.update(Msg::ToggleAnchor);
        assert_eq!(m.anchor, None);
        assert_eq!(m.status.as_ref().unwrap().0, "Range cleared");
    }

    #[test]
    fn a_day_type_key_with_an_anchor_confirms_the_whole_range_in_date_order() {
        let today = d(2026, 9, 18);
        let (mut m, rx) = model(today);
        m.month = Some(month_data(
            today,
            work_days(d(2026, 9, 1), d(2026, 9, 30)),
            None,
        ));
        m.update(Msg::ToggleAnchor); // anchor Fri 18
        m.update(Msg::SelectDay(-4)); // cursor Mon 14
        m.update(Msg::SetKind(DayKind::Vacation));
        assert_eq!(
            m.confirm,
            Some(Confirm::SetKindRange {
                from: d(2026, 9, 14),
                to: d(2026, 9, 18),
                kind: DayKind::Vacation,
            })
        );
        assert_eq!(
            m.confirm_text(m.confirm.as_ref().unwrap()),
            "Set 5 weekdays, 2026-09-14 to 2026-09-18, to vacation?"
        );
        m.update(Msg::ConfirmNo);
        assert_eq!(m.anchor, Some(d(2026, 9, 18)), "declining keeps the anchor");
        m.update(Msg::SetKind(DayKind::Vacation));
        m.update(Msg::ConfirmYes);
        assert!(matches!(
            rx.try_recv().unwrap(),
            StoreCmd::SetKindRange {
                from,
                to,
                kind: DayKind::Vacation
            } if from == d(2026, 9, 14) && to == d(2026, 9, 18)
        ));
        assert_eq!(m.anchor, None, "applying clears the anchor");
    }

    #[test]
    fn esc_clears_the_anchor_before_anything_else() {
        let today = d(2026, 9, 15);
        let (mut m, _rx) = model(today);
        m.month = Some(month_data(
            today,
            work_days(d(2026, 9, 1), d(2026, 9, 30)),
            None,
        ));
        m.update(Msg::ToggleAnchor);
        m.update(Msg::Back);
        assert_eq!(m.anchor, None);
        assert_eq!(m.screen, Screen::Month);
    }

    #[test]
    fn a_range_over_a_day_with_entries_is_left_to_the_worker() {
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
        let mut days = work_days(d(2026, 9, 1), d(2026, 9, 30));
        days.iter_mut().find(|dd| dd.date == today).unwrap().entries = vec![e];
        m.month = Some(month_data(today, days, None));
        m.update(Msg::ToggleAnchor);
        m.update(Msg::SetKind(DayKind::Sick));
        assert_eq!(
            m.confirm,
            Some(Confirm::SetKindRange {
                from: today,
                to: today,
                kind: DayKind::Sick,
            }),
            "the single-day pre-checks do not stand in a range's way"
        );
        m.update(Msg::ConfirmYes);
        assert!(matches!(
            rx.try_recv().unwrap(),
            StoreCmd::SetKindRange { .. }
        ));
    }

    // --- projects screen ---

    fn projects_data() -> ProjectsData {
        let p = |id, name: &str, archived| crate::core::Project {
            id,
            name: name.into(),
            color_index: 0,
            archived,
        };
        ProjectsData {
            projects: vec![p(1, "Alpha", false), p(2, "Old", true)],
            net_by_project: std::collections::BTreeMap::from([("Alpha".to_string(), Minutes(600))]),
        }
    }

    #[test]
    fn shift_p_opens_the_projects_screen_and_loads_it() {
        let (mut m, rx) = model(d(2026, 9, 15));
        m.update(Msg::OpenProjects);
        assert_eq!(m.screen, Screen::Projects);
        assert!(matches!(rx.try_recv().unwrap(), StoreCmd::LoadProjects));
    }

    #[test]
    fn a_flips_the_archived_flag_of_the_highlighted_project() {
        let (mut m, rx) = model(d(2026, 9, 15));
        m.update(Msg::OpenProjects);
        let _ = rx.try_recv();
        m.on_store(StoreReply::Projects(projects_data()));
        m.update(Msg::ProjectsSelect(1));
        m.update(Msg::ProjectsToggleArchive);
        assert!(matches!(
            rx.try_recv().unwrap(),
            StoreCmd::ArchiveProject {
                name,
                archived: false
            } if name == "Old"
        ));
    }

    #[test]
    fn the_month_help_lists_the_range_key() {
        let (m, _rx) = model(d(2026, 9, 15));
        assert!(
            m.help_keys()
                .contains(&("V", "start / clear a range for the day-type keys"))
        );
        assert!(
            !m.key_hints().iter().any(|(k, _)| *k == "V"),
            "the day hint row has no room for it"
        );
    }

    #[test]
    fn r_renames_through_the_name_box_and_blank_names_are_refused() {
        let (mut m, rx) = model(d(2026, 9, 15));
        m.update(Msg::OpenProjects);
        let _ = rx.try_recv();
        m.on_store(StoreReply::Projects(projects_data()));
        m.update(Msg::ProjectsRename);
        assert_eq!(
            m.prompt,
            Some(PromptKind::RenameProject {
                old: "Alpha".into()
            })
        );
        m.update(Msg::PromptSubmit("   ".into()));
        assert!(m.prompt.is_some(), "blank name keeps the box open");
        assert!(
            m.status
                .as_ref()
                .is_some_and(|s| s.0.contains("must not be empty"))
        );
        // The same name is no rename at all: the box just closes.
        m.update(Msg::PromptSubmit("Alpha".into()));
        assert_eq!(m.prompt, None);
        assert!(rx.try_recv().is_err());
        m.update(Msg::ProjectsRename);
        m.update(Msg::PromptSubmit("Alpha2".into()));
        assert_eq!(m.prompt, None);
        assert!(matches!(
            rx.try_recv().unwrap(),
            StoreCmd::RenameProject { old, new } if old == "Alpha" && new == "Alpha2"
        ));
    }

    #[test]
    fn n_adds_a_project_through_the_name_box() {
        let (mut m, rx) = model(d(2026, 9, 15));
        m.update(Msg::OpenProjects);
        let _ = rx.try_recv();
        m.update(Msg::ProjectsAdd);
        assert_eq!(m.prompt, Some(PromptKind::AddProject));
        m.update(Msg::PromptSubmit("Gamma".into()));
        assert!(matches!(rx.try_recv().unwrap(), StoreCmd::AddProject(n) if n == "Gamma"));
        // Esc on the box leaves nothing behind.
        m.update(Msg::ProjectsAdd);
        m.update(Msg::PromptCancel);
        assert_eq!(m.prompt, None);
    }

    #[test]
    fn a_write_reloads_the_projects_screen_and_keeps_the_cursor() {
        let (mut m, rx) = model(d(2026, 9, 15));
        m.update(Msg::OpenProjects);
        let _ = rx.try_recv();
        m.on_store(StoreReply::Projects(projects_data()));
        m.update(Msg::ProjectsSelect(1));
        m.on_store(StoreReply::Changed("Old unarchived".into()));
        assert!(
            std::iter::from_fn(|| rx.try_recv().ok()).any(|c| matches!(c, StoreCmd::LoadProjects)),
            "the screen reloads itself after a write"
        );
        // A shorter list clamps the cursor instead of pointing past the end.
        let mut shorter = projects_data();
        shorter.projects.truncate(1);
        m.on_store(StoreReply::Projects(shorter));
        assert_eq!(m.projects_cursor, 0);
    }

    #[test]
    fn leaving_the_projects_screen_reloads_the_month() {
        let (mut m, rx) = model(d(2026, 9, 15));
        m.update(Msg::OpenProjects);
        let _ = rx.try_recv();
        m.update(Msg::Back);
        assert_eq!(m.screen, Screen::Month);
        assert!(matches!(rx.try_recv().unwrap(), StoreCmd::LoadMonth { .. }));
    }

    #[test]
    fn the_projects_hints_and_help_name_its_keys() {
        let (mut m, _rx) = model(d(2026, 9, 15));
        m.screen = Screen::Projects;
        assert!(m.key_hints().contains(&("a", "archive")));
        assert!(m.help_keys().contains(&("a", "archive / unarchive")));
        m.screen = Screen::Month;
        assert!(
            m.help_keys()
                .contains(&("P", "projects: archive, rename, add"))
        );
    }
}
