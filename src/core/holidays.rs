use chrono::{Datelike, Days, NaiveDate, Weekday};

/// Gregorian computus (Meeus/Jones/Butcher).
pub fn easter_sunday(year: i32) -> NaiveDate {
    let a = year % 19;
    let b = year / 100;
    let c = year % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = (h + l - 7 * m + 114) / 31;
    let day = ((h + l - 7 * m + 114) % 31) + 1;
    NaiveDate::from_ymd_opt(year, month as u32, day as u32).expect("valid easter date")
}

/// Wednesday before 23 November.
pub fn buss_und_bettag(year: i32) -> NaiveDate {
    let mut d = NaiveDate::from_ymd_opt(year, 11, 22).unwrap();
    while d.weekday() != Weekday::Wed {
        d = d.pred_opt().unwrap();
    }
    d
}

pub fn saxony_holidays(year: i32) -> Vec<(NaiveDate, &'static str)> {
    let ymd = |m, d| NaiveDate::from_ymd_opt(year, m, d).unwrap();
    let easter = easter_sunday(year);
    let off = |n: u64, back: bool| {
        if back {
            easter.checked_sub_days(Days::new(n)).unwrap()
        } else {
            easter.checked_add_days(Days::new(n)).unwrap()
        }
    };
    let mut v = vec![
        (ymd(1, 1), "Neujahr"),
        (off(2, true), "Karfreitag"),
        (off(1, false), "Ostermontag"),
        (ymd(5, 1), "Tag der Arbeit"),
        (off(39, false), "Christi Himmelfahrt"),
        (off(50, false), "Pfingstmontag"),
        (ymd(10, 3), "Tag der Deutschen Einheit"),
        (ymd(10, 31), "Reformationstag"),
        (buss_und_bettag(year), "Buß- und Bettag"),
        (ymd(12, 25), "1. Weihnachtstag"),
        (ymd(12, 26), "2. Weihnachtstag"),
    ];
    v.sort();
    v
}

#[derive(Clone, Debug, Default)]
pub struct HolidayCalendar {
    extra: Vec<NaiveDate>,
}

impl HolidayCalendar {
    pub fn new(extra: Vec<NaiveDate>) -> Self {
        Self { extra }
    }

    pub fn holiday_name(&self, d: NaiveDate) -> Option<String> {
        if self.extra.contains(&d) {
            return Some("Extra holiday".to_string());
        }
        saxony_holidays(d.year())
            .into_iter()
            .find(|(hd, _)| *hd == d)
            .map(|(_, n)| n.to_string())
    }

    pub fn is_holiday(&self, d: NaiveDate) -> bool {
        self.holiday_name(d).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easter_known_dates() {
        let d = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
        assert_eq!(easter_sunday(2024), d(2024, 3, 31));
        assert_eq!(easter_sunday(2025), d(2025, 4, 20));
        assert_eq!(easter_sunday(2026), d(2026, 4, 5));
        assert_eq!(easter_sunday(2030), d(2030, 4, 21));
    }

    #[test]
    fn matches_fixture_2024_to_2030() {
        let fixture = include_str!("../../tests/fixtures/saxony_holidays.txt");
        let mut expected: Vec<(NaiveDate, String)> = fixture
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let (d, n) = l.split_once(' ').unwrap();
                (
                    NaiveDate::parse_from_str(d, "%Y-%m-%d").unwrap(),
                    n.to_string(),
                )
            })
            .collect();
        expected.sort();
        let mut actual: Vec<(NaiveDate, String)> = (2024..=2030)
            .flat_map(saxony_holidays)
            .map(|(d, n)| (d, n.to_string()))
            .collect();
        actual.sort();
        assert_eq!(actual, expected);
    }

    #[test]
    fn eleven_per_year_and_no_fronleichnam() {
        for y in 2020..=2040 {
            let h = saxony_holidays(y);
            assert_eq!(h.len(), 11, "year {y}");
            assert!(h.iter().all(|(_, n)| *n != "Fronleichnam"));
        }
    }

    #[test]
    fn calendar_with_extras() {
        let extra = NaiveDate::from_ymd_opt(2026, 12, 24).unwrap();
        let cal = HolidayCalendar::new(vec![extra]);
        assert_eq!(cal.holiday_name(extra).as_deref(), Some("Extra holiday"));
        assert_eq!(
            cal.holiday_name(NaiveDate::from_ymd_opt(2026, 11, 18).unwrap())
                .as_deref(),
            Some("Buß- und Bettag")
        );
        assert!(!cal.is_holiday(NaiveDate::from_ymd_opt(2026, 9, 15).unwrap()));
    }
}
