//! module

pub mod breaks;
pub mod error;
pub mod minutes;
pub mod time_parse;
pub mod types;

pub use breaks::{BreakTier, deduction, default_tiers};
pub use error::CoreError;
pub use minutes::Minutes;
pub use time_parse::{parse_date, parse_time, parse_time_range};
pub use types::{Day, DayKind, Entry, Project, minutes_of};
