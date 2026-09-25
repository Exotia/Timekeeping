//! Component ids mounted in the tui-realm `Application`.

#[derive(Debug, Eq, PartialEq, Clone, Hash)]
pub enum Id {
    Bridge,
    Month,
    Day,
    Form,
    Stats,
    Settings,
    BreakSplit,
    ClockPicker,
    Confirm,
    Help,
    // --- projects screen ---
    Projects,
    Prompt,
    // --- import picker ---
    FilePicker,
}
