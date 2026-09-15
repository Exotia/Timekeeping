//! module

pub mod error;
pub mod minutes;
pub mod time_parse;
pub mod types;

pub use error::CoreError;
pub use minutes::Minutes;
pub use time_parse::{parse_date, parse_time, parse_time_range};
pub use types::{Day, DayKind, Entry, Project, minutes_of};
