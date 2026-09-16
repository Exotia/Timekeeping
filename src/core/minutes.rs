use std::fmt;
use std::iter::Sum;
use std::ops::{Add, AddAssign, Neg, Sub};
use std::str::FromStr;

use super::error::CoreError;

/// How a duration is written on screen: `07:48` or `7.80h`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum HoursFormat {
    /// `+07:48` — hours and minutes.
    #[default]
    Hm,
    /// `+7.80h` — hours with two decimals.
    Decimal,
}

impl HoursFormat {
    /// The spelling `config.toml` uses.
    pub fn as_str(self) -> &'static str {
        match self {
            HoursFormat::Hm => "hm",
            HoursFormat::Decimal => "decimal",
        }
    }

    pub fn parse(s: &str) -> Option<HoursFormat> {
        match s {
            "hm" => Some(HoursFormat::Hm),
            "decimal" => Some(HoursFormat::Decimal),
            _ => None,
        }
    }

    pub fn toggle(self) -> HoursFormat {
        match self {
            HoursFormat::Hm => HoursFormat::Decimal,
            HoursFormat::Decimal => HoursFormat::Hm,
        }
    }
}

/// Signed duration in whole minutes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Minutes(pub i32);

impl Minutes {
    pub const ZERO: Minutes = Minutes(0);

    pub fn hm(hours: i32, minutes: i32) -> Self {
        Minutes(hours * 60 + minutes)
    }

    pub fn is_negative(self) -> bool {
        self.0 < 0
    }

    pub fn abs(self) -> Self {
        Minutes(self.0.abs())
    }

    /// Unsigned "HH:MM" without sign, used inside sentences.
    pub fn hhmm(self) -> String {
        let v = self.0.abs();
        format!("{:02}:{:02}", v / 60, v % 60)
    }

    /// The signed form the display setting asks for: `+07:48` or `+7.80h`.
    pub fn fmt_signed(self, f: HoursFormat) -> String {
        match f {
            HoursFormat::Hm => self.to_string(),
            HoursFormat::Decimal => {
                let sign = if self.0 < 0 { '-' } else { '+' };
                format!("{sign}{}", self.fmt_unsigned(f))
            }
        }
    }

    /// The unsigned form the display setting asks for: `07:48` or `7.80h`.
    ///
    /// Used where the sign is already in the sentence around it ("break -00:48").
    pub fn fmt_unsigned(self, f: HoursFormat) -> String {
        match f {
            HoursFormat::Hm => self.hhmm(),
            HoursFormat::Decimal => format!("{:.2}h", self.0.abs() as f64 / 60.0),
        }
    }
}

impl fmt::Display for Minutes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = if self.0 < 0 { '-' } else { '+' };
        write!(f, "{sign}{}", self.hhmm())
    }
}

impl FromStr for Minutes {
    type Err = CoreError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (sign, body) = match s.as_bytes().first() {
            Some(b'-') => (-1, &s[1..]),
            Some(b'+') => (1, &s[1..]),
            _ => (1, s),
        };
        let (h, m) = body
            .split_once(':')
            .ok_or_else(|| CoreError::InvalidTime(s.to_string()))?;
        let h: i32 = h
            .parse()
            .map_err(|_| CoreError::InvalidTime(s.to_string()))?;
        let m: i32 = m
            .parse()
            .map_err(|_| CoreError::InvalidTime(s.to_string()))?;
        if !(0..60).contains(&m) || h < 0 {
            return Err(CoreError::InvalidTime(s.to_string()));
        }
        Ok(Minutes(sign * (h * 60 + m)))
    }
}

impl Add for Minutes {
    type Output = Minutes;
    fn add(self, rhs: Minutes) -> Minutes {
        Minutes(self.0 + rhs.0)
    }
}
impl Sub for Minutes {
    type Output = Minutes;
    fn sub(self, rhs: Minutes) -> Minutes {
        Minutes(self.0 - rhs.0)
    }
}
impl Neg for Minutes {
    type Output = Minutes;
    fn neg(self) -> Minutes {
        Minutes(-self.0)
    }
}
impl AddAssign for Minutes {
    fn add_assign(&mut self, rhs: Minutes) {
        self.0 += rhs.0;
    }
}
impl Sum for Minutes {
    fn sum<I: Iterator<Item = Minutes>>(iter: I) -> Minutes {
        iter.fold(Minutes::ZERO, |a, b| a + b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displays_signed_hh_mm() {
        assert_eq!(Minutes(0).to_string(), "+00:00");
        assert_eq!(Minutes(468).to_string(), "+07:48");
        assert_eq!(Minutes(-594).to_string(), "-09:54");
        assert_eq!(Minutes(6000).to_string(), "+100:00");
    }

    #[test]
    fn parses_signed_and_unsigned() {
        assert_eq!("+07:48".parse::<Minutes>().unwrap(), Minutes(468));
        assert_eq!("-00:30".parse::<Minutes>().unwrap(), Minutes(-30));
        assert_eq!("7:48".parse::<Minutes>().unwrap(), Minutes(468));
        assert!("abc".parse::<Minutes>().is_err());
        assert!("07:60".parse::<Minutes>().is_err());
    }

    #[test]
    fn arithmetic() {
        assert_eq!(Minutes(10) + Minutes(5), Minutes(15));
        assert_eq!(Minutes(10) - Minutes(15), Minutes(-5));
        assert_eq!(-Minutes(3), Minutes(-3));
        assert_eq!(Minutes::hm(7, 48), Minutes(468));
        let total: Minutes = [Minutes(1), Minutes(2)].into_iter().sum();
        assert_eq!(total, Minutes(3));
        assert!(Minutes(-1).is_negative());
        assert_eq!(Minutes(-1).abs(), Minutes(1));
    }

    #[test]
    fn formats_in_either_hours_format() {
        use HoursFormat::{Decimal, Hm};
        assert_eq!(Minutes(468).fmt_signed(Hm), "+07:48");
        assert_eq!(Minutes(468).fmt_signed(Decimal), "+7.80h");
        assert_eq!(Minutes(468).fmt_unsigned(Hm), "07:48");
        assert_eq!(Minutes(468).fmt_unsigned(Decimal), "7.80h");
        assert_eq!(Minutes(-30).fmt_signed(Decimal), "-0.50h");
        assert_eq!(Minutes(-30).fmt_unsigned(Decimal), "0.50h");
        assert_eq!(Minutes::ZERO.fmt_signed(Decimal), "+0.00h");
        assert_eq!(Minutes(6000).fmt_signed(Decimal), "+100.00h");
        assert_eq!(Minutes(6000).fmt_signed(Hm), "+100:00");
        // `Display` and `hhmm` stay h:mm whatever the setting says.
        assert_eq!(Minutes(468).to_string(), "+07:48");
        assert_eq!(Minutes(468).hhmm(), "07:48");
    }

    #[test]
    fn hours_format_parses_and_toggles() {
        use HoursFormat::{Decimal, Hm};
        assert_eq!(HoursFormat::default(), Hm);
        assert_eq!(Hm.as_str(), "hm");
        assert_eq!(Decimal.as_str(), "decimal");
        for f in [Hm, Decimal] {
            assert_eq!(HoursFormat::parse(f.as_str()), Some(f));
        }
        assert_eq!(HoursFormat::parse("HM"), None);
        assert_eq!(HoursFormat::parse(""), None);
        assert_eq!(Hm.toggle(), Decimal);
        assert_eq!(Decimal.toggle(), Hm);
    }
}
