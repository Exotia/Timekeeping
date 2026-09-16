//! Messages exchanged between components, the model and the store worker.

use chrono::{NaiveDate, NaiveTime};

use crate::core::{Day, DayKind, Minutes, Project};
use crate::store::Session;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum RangeKind {
    ThisMonth,
    LastMonth,
    Quarter,
    Year,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Confirm {
    DeleteEntry(i64),
    SetKind(NaiveDate, DayKind),
}

#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub struct FormData {
    pub id: Option<i64>,
    pub start: String,
    pub end: String,
    pub project: String,
    pub comment: String,
}

/// The four raw field values of the settings overlay, exactly as typed.
#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub struct SettingsData {
    pub start: String,
    pub balance: String,
    pub target: String,
    pub vacation: String,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Msg {
    Quit,
    Tick,
    Store(StoreReply),
    Error(String),
    Info(String),
    /// Move the selection by N days.
    SelectDay(i32),
    /// Move the selection by N months.
    SelectMonth(i32),
    GoToday,
    OpenDay,
    OpenStats,
    Back,
    /// `i`: pick the project to clock in on, or to switch to.
    OpenClockPicker,
    /// The highlight or the filter in that overlay moved: repaint it.
    ClockPickerChanged,
    /// The project chosen in that overlay.
    ClockPickerSubmit(String),
    ClockPickerCancel,
    ClockOut,
    SetKind(DayKind),
    AskConfirm(Confirm),
    ConfirmYes,
    ConfirmNo,
    ToggleHelp,
    // day editor
    DaySelect(i32),
    DayAdd,
    DayEdit,
    DayDelete,
    DayKindNext,
    DayKindPrev,
    // form
    FormSubmit(FormData),
    FormCancel,
    /// A form field changed; carries the whole form so the model can re-validate.
    FormChanged(FormData),
    // settings
    OpenSettings,
    /// A settings field changed; carries the whole overlay so the model can re-validate.
    SettingsChanged(SettingsData),
    SettingsSubmit(SettingsData),
    SettingsCancel,
    // stats
    StatsRange(RangeKind),
}

#[derive(Debug, Clone)]
pub enum StoreCmd {
    LoadMonth {
        year: i32,
        month: u32,
    },
    LoadDay(NaiveDate),
    LoadStats {
        from: NaiveDate,
        to: NaiveDate,
    },
    AddEntry {
        date: NaiveDate,
        start: NaiveTime,
        end: NaiveTime,
        project: String,
        comment: String,
    },
    UpdateEntry {
        id: i64,
        start: NaiveTime,
        end: NaiveTime,
        project: String,
        comment: String,
    },
    DeleteEntry(i64),
    SetKind(NaiveDate, DayKind),
    ClockIn(NaiveDate, NaiveTime, String),
    ClockOut {
        project: Option<String>,
        comment: String,
    },
    /// Book the running session and clock in on this project instead.
    Switch {
        project: String,
    },
    Shutdown,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct MonthData {
    pub year: i32,
    pub month: u32,
    pub days: Vec<Day>,
    /// Running balance up to the last day before this month.
    pub balance_before: Minutes,
    /// Running balance through today.
    pub balance_total: Minutes,
    pub session: Option<Session>,
    pub projects: Vec<Project>,
    /// The project of the most recent entry, offered first by the clock-in picker.
    pub last_used_project: Option<String>,
    pub vacation_used_this_year: u32,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct DayData {
    pub day: Day,
    pub projects: Vec<Project>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub struct StatsData {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub days: Vec<Day>,
    pub projects: Vec<Project>,
    /// Vacation working days taken in the whole calendar year of today — the
    /// allowance is a yearly budget, so the remainder must not follow the range.
    pub vacation_used_year: u32,
    /// Whether a clock-in is open right now, for today's target rule.
    pub session_active: bool,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum StoreReply {
    Month(MonthData),
    Day(DayData),
    Stats(StatsData),
    /// Success message; empty when there is nothing to show.
    Changed(String),
    Failed(String),
}

#[derive(Debug, Clone)]
pub enum UserEvent {
    Store(StoreReply),
}

/// Subscriptions match user events by `PartialEq`, so compare discriminants only.
impl PartialEq for UserEvent {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

impl Eq for UserEvent {}
