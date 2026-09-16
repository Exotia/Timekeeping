use chrono::{Days, NaiveDate, NaiveTime, TimeDelta, Timelike};

use super::{CoreError, minutes_of};

/// End time to record when clocking out at `now` for a session started at `start`.
///
/// `now` is truncated to whole minutes, because entries are stored at minute granularity.
/// If that lands on the very minute the session started, one minute is added instead: a
/// zero-length entry would normalise to `end <= start`, i.e. a 24-hour shift.
///
/// Clocking out at exactly 23:59 therefore yields 00:00, which core reads as a one-minute
/// entry crossing midnight (`0 + 1440 - 1439 == 1`).
///
/// Core has no date, so it cannot tell an end *before* the start apart from a session
/// that ran overnight: that judgement belongs to the store, which knows the session's
/// date, and it clamps the end before calling this.
pub fn clock_out_end(start: NaiveTime, now: NaiveTime) -> NaiveTime {
    let end = NaiveTime::from_hms_opt(now.hour(), now.minute(), 0).unwrap_or(now);
    if minutes_of(end) == minutes_of(start) {
        return end.overflowing_add_signed(TimeDelta::minutes(1)).0;
    }
    end
}

pub fn parse_time(s: &str) -> Result<NaiveTime, CoreError> {
    let err = || CoreError::InvalidTime(s.to_string());
    let s = s.trim();
    if s.is_empty() {
        return Err(err());
    }
    let (h, m): (u32, u32) = if let Some((h, m)) = s.split_once([':', '.']) {
        (h.parse().map_err(|_| err())?, m.parse().map_err(|_| err())?)
    } else {
        if !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(err());
        }
        match s.len() {
            1 | 2 => (s.parse().map_err(|_| err())?, 0),
            3 | 4 => {
                let (h, m) = s.split_at(s.len() - 2);
                (h.parse().map_err(|_| err())?, m.parse().map_err(|_| err())?)
            }
            _ => return Err(err()),
        }
    };
    NaiveTime::from_hms_opt(h, m, 0).ok_or_else(err)
}

/// The one rule every entry range must obey: the two ends must differ.
///
/// `Entry::interval` reads `end <= start` as crossing midnight and adds a day, so an
/// equal pair would silently become a 24-hour entry. `end < start` is a genuine
/// crossing and stays valid. This is the single check shared by `parse_time_range`,
/// the TUI entry form and the store's write paths.
pub fn check_range(start: NaiveTime, end: NaiveTime) -> Result<(), CoreError> {
    if minutes_of(start) == minutes_of(end) {
        return Err(CoreError::InvalidRange);
    }
    Ok(())
}

pub fn parse_time_range(s: &str) -> Result<(NaiveTime, NaiveTime), CoreError> {
    let (a, b) = s
        .split_once('-')
        .ok_or_else(|| CoreError::InvalidTime(s.to_string()))?;
    let (start, end) = (parse_time(a)?, parse_time(b)?);
    check_range(start, end)?;
    Ok((start, end))
}

pub fn parse_date(s: &str, today: NaiveDate) -> Result<NaiveDate, CoreError> {
    let err = || CoreError::InvalidDate(s.to_string());
    let s = s.trim();
    match s.to_ascii_lowercase().as_str() {
        "today" => return Ok(today),
        "yesterday" => return today.pred_opt().ok_or_else(err),
        "tomorrow" => return today.succ_opt().ok_or_else(err),
        _ => {}
    }
    if let Ok(off) = s.parse::<i64>() {
        return if off >= 0 {
            today
                .checked_add_days(Days::new(off as u64))
                .ok_or_else(err)
        } else {
            today
                .checked_sub_days(Days::new(off.unsigned_abs()))
                .ok_or_else(err)
        };
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| err())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveTime};

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn clock_out_end_never_produces_a_zero_length_entry() {
        // (a) same minute as the start, with the sub-minute parts `Local::now()` carries.
        let start = t(9, 0);
        let now = NaiveTime::from_hms_nano_opt(9, 0, 42, 123_456).unwrap();
        assert_eq!(clock_out_end(start, now), t(9, 1));
        // (b) later: truncated to the minute.
        let now = NaiveTime::from_hms_nano_opt(15, 29, 59, 999_999_999).unwrap();
        assert_eq!(clock_out_end(start, now), t(15, 29));
        // (c) the last minute of the day wraps to 00:00, which core reads as one minute.
        let start = t(23, 59);
        let now = NaiveTime::from_hms_nano_opt(23, 59, 30, 0).unwrap();
        assert_eq!(clock_out_end(start, now), t(0, 0));
    }

    #[test]
    fn parses_time_forms() {
        for (s, exp) in [
            ("8", t(8, 0)),
            ("800", t(8, 0)),
            ("0800", t(8, 0)),
            ("8:00", t(8, 0)),
            ("17:30", t(17, 30)),
            ("1730", t(17, 30)),
            ("17.30", t(17, 30)),
            ("0", t(0, 0)),
            ("2359", t(23, 59)),
        ] {
            assert_eq!(parse_time(s).unwrap(), exp, "input {s}");
        }
        for bad in ["", "abc", "2400", "1260", "12:60", "123456", "-1"] {
            assert!(parse_time(bad).is_err(), "input {bad}");
        }
    }

    #[test]
    fn parses_dates() {
        let today = NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
        assert_eq!(parse_date("today", today).unwrap(), today);
        assert_eq!(
            parse_date("yesterday", today).unwrap(),
            today.pred_opt().unwrap()
        );
        assert_eq!(
            parse_date("tomorrow", today).unwrap(),
            today.succ_opt().unwrap()
        );
        assert_eq!(parse_date("-1", today).unwrap(), today.pred_opt().unwrap());
        assert_eq!(
            parse_date("+2", today).unwrap(),
            NaiveDate::from_ymd_opt(2026, 9, 17).unwrap()
        );
        assert_eq!(parse_date("0", today).unwrap(), today);
        assert_eq!(
            parse_date("2026-01-31", today).unwrap(),
            NaiveDate::from_ymd_opt(2026, 1, 31).unwrap()
        );
        assert!(parse_date("2026-02-30", today).is_err());
        assert!(parse_date("nope", today).is_err());
    }

    #[test]
    fn parses_range() {
        assert_eq!(parse_time_range("0900-1530").unwrap(), (t(9, 0), t(15, 30)));
        assert_eq!(
            parse_time_range("9:00-17:30").unwrap(),
            (t(9, 0), t(17, 30))
        );
        assert!(parse_time_range("0900").is_err());
        // An earlier end crosses midnight and stays valid.
        assert_eq!(parse_time_range("2200-0200").unwrap(), (t(22, 0), t(2, 0)));
    }

    #[test]
    fn a_range_with_equal_ends_is_rejected() {
        // `Entry::interval` would read it as a 24-hour shift.
        assert_eq!(
            parse_time_range("0900-0900"),
            Err(CoreError::InvalidRange),
            "0900-0900 must not become a 24-hour entry"
        );
        assert_eq!(parse_time_range("9-9:00"), Err(CoreError::InvalidRange));
        assert_eq!(check_range(t(9, 0), t(9, 0)), Err(CoreError::InvalidRange));
        assert!(check_range(t(22, 0), t(2, 0)).is_ok());
        assert!(check_range(t(9, 0), t(9, 1)).is_ok());
    }
}
