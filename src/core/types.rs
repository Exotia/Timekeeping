use chrono::{NaiveDate, NaiveTime, Timelike};

use super::Minutes;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DayKind {
    Work,
    Vacation,
    Flex,
    Holiday,
    Sick,
    Absence { label: String },
}

impl DayKind {
    pub const ALL_NAMES: [&'static str; 6] =
        ["work", "vacation", "flex", "holiday", "sick", "absence"];

    pub fn as_str(&self) -> &'static str {
        match self {
            DayKind::Work => "work",
            DayKind::Vacation => "vacation",
            DayKind::Flex => "flex",
            DayKind::Holiday => "holiday",
            DayKind::Sick => "sick",
            DayKind::Absence { .. } => "absence",
        }
    }

    pub fn parse(kind: &str, label: Option<&str>) -> Option<DayKind> {
        Some(match kind.to_ascii_lowercase().as_str() {
            "work" => DayKind::Work,
            "vacation" => DayKind::Vacation,
            "flex" => DayKind::Flex,
            "holiday" => DayKind::Holiday,
            "sick" => DayKind::Sick,
            "absence" => DayKind::Absence {
                label: label.unwrap_or("absence").to_string(),
            },
            _ => return None,
        })
    }

    /// Whether the daily target is deducted on a weekday of this kind.
    pub fn has_target(&self) -> bool {
        matches!(self, DayKind::Work | DayKind::Flex)
    }

    /// Only Work days may carry time entries.
    pub fn allows_entries(&self) -> bool {
        matches!(self, DayKind::Work)
    }

    pub fn display_name(&self) -> String {
        match self {
            DayKind::Work => "Work".into(),
            DayKind::Vacation => "Vacation".into(),
            DayKind::Flex => "Flex day".into(),
            DayKind::Holiday => "Holiday".into(),
            DayKind::Sick => "Sick".into(),
            DayKind::Absence { label } => label.clone(),
        }
    }

    pub fn label(&self) -> Option<&str> {
        match self {
            DayKind::Absence { label } => Some(label),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub id: i64,
    pub date: NaiveDate,
    pub start: NaiveTime,
    pub end: NaiveTime,
    pub project: String,
    pub comment: String,
}

pub fn minutes_of(t: NaiveTime) -> i32 {
    (t.hour() * 60 + t.minute()) as i32
}

impl Entry {
    /// (start, end) in minutes since midnight; end > 1440 when the entry crosses midnight.
    pub fn interval(&self) -> (i32, i32) {
        let s = minutes_of(self.start);
        let mut e = minutes_of(self.end);
        if e <= s {
            e += 1440;
        }
        (s, e)
    }

    pub fn duration(&self) -> Minutes {
        let (s, e) = self.interval();
        Minutes(e - s)
    }

    pub fn crosses_midnight(&self) -> bool {
        minutes_of(self.end) <= minutes_of(self.start)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Day {
    pub date: NaiveDate,
    pub kind: DayKind,
    pub entries: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub color_index: u8,
    pub archived: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveTime};

    fn e(start: (u32, u32), end: (u32, u32)) -> Entry {
        Entry {
            id: 1,
            date: NaiveDate::from_ymd_opt(2026, 9, 14).unwrap(),
            start: NaiveTime::from_hms_opt(start.0, start.1, 0).unwrap(),
            end: NaiveTime::from_hms_opt(end.0, end.1, 0).unwrap(),
            project: "Alpha".into(),
            comment: String::new(),
        }
    }

    #[test]
    fn duration_same_day() {
        assert_eq!(e((9, 0), (15, 30)).duration(), Minutes(390));
        assert_eq!(e((9, 0), (15, 30)).interval(), (540, 930));
        assert!(!e((9, 0), (15, 30)).crosses_midnight());
    }

    #[test]
    fn duration_crossing_midnight() {
        assert_eq!(e((22, 0), (2, 0)).duration(), Minutes(240));
        assert_eq!(e((22, 0), (2, 0)).interval(), (1320, 1560));
        assert!(e((22, 0), (2, 0)).crosses_midnight());
        // end == start is a 24h shift
        assert_eq!(e((8, 0), (8, 0)).duration(), Minutes(1440));
    }

    #[test]
    fn day_kind_roundtrip_and_flags() {
        for k in ["work", "vacation", "flex", "holiday", "sick"] {
            assert_eq!(DayKind::parse(k, None).unwrap().as_str(), k);
        }
        let a = DayKind::parse("absence", Some("training")).unwrap();
        assert_eq!(
            a,
            DayKind::Absence {
                label: "training".into()
            }
        );
        assert_eq!(a.display_name(), "training");
        assert!(DayKind::parse("absence", None).is_some());
        assert!(DayKind::parse("party", None).is_none());
        assert!(DayKind::Work.has_target());
        assert!(DayKind::Flex.has_target());
        assert!(!DayKind::Vacation.has_target());
        assert!(!DayKind::Holiday.has_target());
        assert!(!DayKind::Sick.has_target());
        assert!(DayKind::Work.allows_entries());
        assert!(!DayKind::Flex.allows_entries());
        assert_eq!(DayKind::Vacation.display_name(), "Vacation");
    }
}
