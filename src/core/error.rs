use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum CoreError {
    #[error("entry overlaps an existing entry")]
    Overlap,
    #[error("invalid time: {0}")]
    InvalidTime(String),
    #[error("invalid date: {0}")]
    InvalidDate(String),
    #[error("end must differ from start (an earlier end crosses midnight)")]
    InvalidRange,
}
