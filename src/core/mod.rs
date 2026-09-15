//! module

pub mod error;
pub mod minutes;
pub mod types;

pub use error::CoreError;
pub use minutes::Minutes;
pub use types::{Day, DayKind, Entry, Project, minutes_of};
