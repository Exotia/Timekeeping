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
    ClockInReplace,
}

#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub struct FormData {
    pub id: Option<i64>,
    pub start: String,
    pub end: String,
    pub project: String,
    pub comment: String,
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
    ClockIn,
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
    ClockIn(NaiveDate, NaiveTime),
    ClockOut {
        project: Option<String>,
        comment: String,
    },
    ClearSession,
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
