//! module

pub mod balance;
pub mod breaks;
pub mod error;
pub mod holidays;
pub mod minutes;
pub mod time_parse;
pub mod types;

pub use balance::{
    DayStats, Rules, TodayCtx, check_overlap, day_stats, effective_kind, is_working_day,
    provisional_net, provisional_net_for, running_balance, running_minutes,
};
pub use breaks::{BreakTier, day_deduction, deduction, default_tiers, gaps_between, recorded_gaps};
pub use error::CoreError;
pub use holidays::{HolidayCalendar, easter_sunday, saxony_holidays};
pub use minutes::{HoursFormat, Minutes};
pub use time_parse::{check_range, clock_out_end, parse_date, parse_time, parse_time_range};
pub use types::{Day, DayKind, Entry, Project, minutes_of};
