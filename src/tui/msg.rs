//! Messages exchanged between components, the model and the store worker.

use std::collections::BTreeMap;

use chrono::{NaiveDate, NaiveTime};

use crate::core::{Day, DayKind, Minutes, Project};
use crate::store::Session;

/// A period length. Which period is on screen follows the statistics screen's
/// anchor date, which `[`, `]` and `t` move.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum RangeKind {
    /// The ISO week (Monday … Sunday) the anchor falls in.
    Week,
    /// The calendar month the anchor falls in.
    Month,
    /// The calendar quarter the anchor falls in.
    Quarter,
    /// The calendar year the anchor falls in.
    Year,
}

/// What the bars of the statistics chart measure. `r` flips between them.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ChartMode {
    /// One bar per period, each the balance that period earned on its own.
    #[default]
    PerPeriod,
    /// One bar per period, each the balance as it stood when that period ended
    /// — the initial balance and everything before the range included.
    Running,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Confirm {
    DeleteEntry(i64),
    SetKind(NaiveDate, DayKind),
    // --- range marking ---
    SetKindRange {
        from: NaiveDate,
        to: NaiveDate,
        kind: DayKind,
    },
    // --- import key ---
    /// The dry run's counts, shown before anything is written.
    ImportFile {
        path: std::path::PathBuf,
        imported: usize,
        skipped: usize,
    },
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
    /// `b` on the month view: book the work so far and pause the clock.
    TakeBreak,
    SetKind(DayKind),
    AskConfirm(Confirm),
    ConfirmYes,
    ConfirmNo,
    ToggleHelp,
    /// `u`: switch every duration on screen between `h:mm` and decimal hours.
    ToggleHours,
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
    // break split
    /// `b` on an entry, or the box opening by itself after a clock-out: edit the
    /// break shares of the session `entry_id` belongs to.
    OpenBreakSplit {
        date: NaiveDate,
        entry_id: i64,
    },
    /// `b` in the day editor: the same, on the entry under the cursor.
    DayBreakSplit,
    /// A field of the break-split box changed; carries every field as typed, so
    /// the model can re-validate and write the footer.
    BreakSplitChanged(Vec<String>),
    /// The shares to save, one per entry of the session.
    BreakSplitSubmit(Vec<(i64, Option<Minutes>)>),
    BreakSplitCancel,
    // settings
    OpenSettings,
    /// A settings field changed; carries the whole overlay so the model can re-validate.
    SettingsChanged(SettingsData),
    SettingsSubmit(SettingsData),
    SettingsCancel,
    // stats
    StatsRange(RangeKind),
    /// `[` / `]`: move the statistics anchor by N whole periods of the range.
    StatsShift(i32),
    /// `t`: bring the statistics anchor back to today.
    StatsToday,
    /// `r`: switch the chart between the per-period bars and the running balance.
    ToggleChartMode,
    // --- backup key ---
    /// `B` on the month screen: copy the database into `<home>/backups/`.
    Backup,
    /// `E` on the month screen: ask where to write the export.
    ExportPrompt,
    /// `I` on the month screen: ask which file to import.
    ImportPrompt,
    // --- range marking ---
    /// `V`: drop or clear the anchor of a date range for the day-type keys.
    ToggleAnchor,
    // --- projects screen ---
    OpenProjects,
    ProjectsSelect(i32),
    /// `a`: archive the highlighted project, or unarchive it if it is archived.
    ProjectsToggleArchive,
    /// `r`: open the name box prefilled with the highlighted project's name.
    ProjectsRename,
    /// `n`: open an empty name box.
    ProjectsAdd,
    PromptChanged,
    PromptSubmit(String),
    PromptCancel,
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
        /// The calendar year of the anchor, for the vacation budget: the
        /// allowance is a yearly one, so a range in 2025 must count 2025.
        year: i32,
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
    /// Book the work so far and pause the clock on the same project.
    Break,
    /// Go back to work from a break, on `project` or on the remembered one.
    Resume {
        project: Option<String>,
    },
    /// Set (or clear, with `None`) the break shares of a session's entries.
    SetBreakShares(Vec<(i64, Option<Minutes>)>),
    Shutdown,
    // --- backup key ---
    Backup,
    // --- export key ---
    Export {
        path: std::path::PathBuf,
    },
    // --- import key ---
    ImportDryRun {
        path: std::path::PathBuf,
    },
    Import {
        path: std::path::PathBuf,
    },
    // --- range marking ---
    SetKindRange {
        from: NaiveDate,
        to: NaiveDate,
        kind: DayKind,
    },
    // --- projects screen ---
    LoadProjects,
    ArchiveProject {
        name: String,
        archived: bool,
    },
    RenameProject {
        old: String,
        new: String,
    },
    AddProject(String),
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
    /// Vacation working days taken in the whole calendar year the range sits in
    /// — the allowance is a yearly budget, so the remainder must not follow the
    /// range itself, only the year it belongs to.
    pub vacation_used_year: u32,
    /// Whether a clock-in is open right now, for today's target rule.
    pub session_active: bool,
    /// The running balance through the day before `from`: the initial balance
    /// plus every day balance from `start_date` up to the range. The running
    /// chart starts from it, so a range never begins at zero as if nothing had
    /// happened before it.
    pub carried_in: Minutes,
}

// --- projects screen ---
/// Everything the projects screen shows: every project, archived ones
/// included, and the net minutes worked on each from `start_date` to today.
#[derive(Debug, PartialEq, Eq, Clone)]
pub struct ProjectsData {
    pub projects: Vec<Project>,
    pub net_by_project: BTreeMap<String, Minutes>,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum StoreReply {
    Month(MonthData),
    Day(DayData),
    Stats(StatsData),
    /// Success message; empty when there is nothing to show.
    Changed(String),
    /// A [`StoreCmd::ClockOut`] or [`StoreCmd::Break`] that booked an entry: a
    /// `Changed` plus what the break-split box needs to decide whether it is
    /// worth opening — how many projects the booked entry's session spans and
    /// what that session loses to the break.
    Booked {
        entry_id: i64,
        date: NaiveDate,
        session_projects: usize,
        deduction: Minutes,
        /// What a plain `Changed` would have said.
        message: String,
    },
    Failed(String),
    // --- import key ---
    /// A dry run's result, asked about before anything is written.
    Confirm(Confirm),
    // --- projects screen ---
    Projects(ProjectsData),
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
