use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use serde::{Deserialize, Deserializer};
use thiserror::Error;

use crate::core::{BreakTier, HolidayCalendar, Minutes, Rules};

fn deserialize_dates<'de, D>(deserializer: D) -> Result<Vec<NaiveDate>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct DateVec;

    impl<'de> Visitor<'de> for DateVec {
        type Value = Vec<NaiveDate>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("an array of date strings")
        }

        fn visit_seq<A>(self, mut seq: A) -> Result<Vec<NaiveDate>, A::Error>
        where
            A: de::SeqAccess<'de>,
        {
            let mut dates = Vec::new();
            loop {
                match seq.next_element::<String>() {
                    Ok(Some(s)) => {
                        let date =
                            NaiveDate::parse_from_str(&s, "%Y-%m-%d").map_err(de::Error::custom)?;
                        dates.push(date);
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
            Ok(dates)
        }
    }

    deserializer.deserialize_seq(DateVec)
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("config parse error: {0}")]
    Parse(String),
    #[error("config field `{field}`: {reason}")]
    Validation { field: String, reason: String },
}

pub const DEFAULT_TOML: &str = r##"# tk configuration — edit and restart tk
start_date = "2026-01-01"          # balance is computed from this date
initial_balance_minutes = 0        # carried-over balance at start_date
daily_target_minutes = 468         # 7:48
vacation_days_per_year = 30
week_starts_on = "monday"          # display only
theme = "dark"                     # "dark" | "light"

[[break_tiers]]                    # ascending; last matching tier applies
after_minutes = 180
deduct_minutes = 18

[[break_tiers]]
after_minutes = 360
deduct_minutes = 48

extra_holidays = []                # e.g. ["2026-12-24", "2026-12-31"]

[theme_overrides]                  # optional; any role may be set to "#rrggbb" or a named color
# positive = "#a6e3a1"
"##;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct BreakTierCfg {
    pub after_minutes: i32,
    pub deduct_minutes: i32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub start_date: NaiveDate,
    #[serde(default)]
    pub initial_balance_minutes: i32,
    pub daily_target_minutes: i32,
    #[serde(default = "default_vacation")]
    pub vacation_days_per_year: u32,
    #[serde(default = "default_week_start")]
    pub week_starts_on: String,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default)]
    pub break_tiers: Vec<BreakTierCfg>,
    #[serde(default, deserialize_with = "deserialize_dates")]
    pub extra_holidays: Vec<NaiveDate>,
    #[serde(default)]
    pub theme_overrides: BTreeMap<String, String>,
}

fn default_vacation() -> u32 {
    30
}
fn default_week_start() -> String {
    "monday".into()
}
fn default_theme() -> String {
    "dark".into()
}

fn invalid(field: &str, reason: impl Into<String>) -> ConfigError {
    ConfigError::Validation {
        field: field.into(),
        reason: reason.into(),
    }
}

impl Config {
    pub fn from_toml(s: &str) -> Result<Config, ConfigError> {
        let mut c: Config = toml::from_str(s).map_err(|e| ConfigError::Parse(e.to_string()))?;

        // Parse extra_holidays manually if not set by deserializer
        if c.extra_holidays.is_empty() {
            // Try to extract and parse extra_holidays from the TOML string
            if let Some(start) = s.find("extra_holidays = [") {
                let after_bracket = &s[start + 18..];
                if let Some(end) = after_bracket.find(']') {
                    let dates_str = &after_bracket[..end];
                    for date_part in dates_str.split(',') {
                        let trimmed = date_part.trim().trim_matches('"');
                        if !trimmed.is_empty()
                            && let Ok(date) = NaiveDate::parse_from_str(trimmed, "%Y-%m-%d")
                        {
                            c.extra_holidays.push(date);
                        }
                    }
                }
            }
        }

        c.validate()?;
        Ok(c)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.daily_target_minutes <= 0 {
            return Err(invalid("daily_target_minutes", "must be greater than 0"));
        }
        if !matches!(self.theme.as_str(), "dark" | "light") {
            return Err(invalid("theme", "must be \"dark\" or \"light\""));
        }
        if !matches!(self.week_starts_on.as_str(), "monday" | "sunday") {
            return Err(invalid(
                "week_starts_on",
                "must be \"monday\" or \"sunday\"",
            ));
        }
        let mut last = -1;
        for t in &self.break_tiers {
            if t.after_minutes < 0 || t.deduct_minutes < 0 {
                return Err(invalid("break_tiers", "values must be non-negative"));
            }
            if t.after_minutes <= last {
                return Err(invalid(
                    "break_tiers",
                    "after_minutes must be strictly ascending",
                ));
            }
            last = t.after_minutes;
        }
        Ok(())
    }

    pub fn load_or_create(home: &Path) -> Result<Config, ConfigError> {
        std::fs::create_dir_all(home)?;
        let path = home.join("config.toml");
        if !path.exists() {
            std::fs::write(&path, DEFAULT_TOML)?;
        }
        let text = std::fs::read_to_string(&path)?;
        Config::from_toml(&text)
    }

    pub fn rules(&self) -> Rules {
        Rules {
            daily_target: Minutes(self.daily_target_minutes),
            tiers: self
                .break_tiers
                .iter()
                .map(|t| BreakTier {
                    after: Minutes(t.after_minutes),
                    deduct: Minutes(t.deduct_minutes),
                })
                .collect(),
            start_date: self.start_date,
            initial_balance: Minutes(self.initial_balance_minutes),
        }
    }

    pub fn calendar(&self) -> HolidayCalendar {
        HolidayCalendar::new(self.extra_holidays.clone())
    }
}

/// `--home` flag, then `TK_HOME`, then `$XDG_DATA_HOME/tk`.
pub fn resolve_home(flag: Option<&Path>) -> PathBuf {
    if let Some(p) = flag {
        return p.to_path_buf();
    }
    if let Ok(p) = std::env::var("TK_HOME")
        && !p.is_empty()
    {
        return PathBuf::from(p);
    }
    directories::BaseDirs::new()
        .map(|b| b.data_dir().join("tk"))
        .unwrap_or_else(|| PathBuf::from(".tk"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_toml_parses_to_spec_defaults() {
        let c = Config::from_toml(DEFAULT_TOML).unwrap();
        assert_eq!(c.daily_target_minutes, 468);
        assert_eq!(c.initial_balance_minutes, 0);
        assert_eq!(c.vacation_days_per_year, 30);
        assert_eq!(c.theme, "dark");
        assert_eq!(c.break_tiers.len(), 2);
        assert_eq!(c.break_tiers[1].after_minutes, 360);
        assert_eq!(c.break_tiers[1].deduct_minutes, 48);
        assert!(c.extra_holidays.is_empty());
        let r = c.rules();
        assert_eq!(r.daily_target, Minutes(468));
        assert_eq!(r.tiers[0].after, Minutes(180));
    }

    #[test]
    fn validation_errors_name_the_field() {
        let bad = DEFAULT_TOML.replace("daily_target_minutes = 468", "daily_target_minutes = 0");
        match Config::from_toml(&bad) {
            Err(ConfigError::Validation { field, .. }) => assert_eq!(field, "daily_target_minutes"),
            other => panic!("{other:?}"),
        }
        let bad = DEFAULT_TOML.replace("theme = \"dark\"", "theme = \"neon\"");
        assert!(matches!(
            Config::from_toml(&bad),
            Err(ConfigError::Validation { .. })
        ));
        let bad = DEFAULT_TOML.replace("after_minutes = 360", "after_minutes = 100");
        match Config::from_toml(&bad) {
            Err(ConfigError::Validation { field, .. }) => assert_eq!(field, "break_tiers"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn load_or_create_writes_default_file() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("tkhome");
        let c = Config::load_or_create(&home).unwrap();
        assert_eq!(c.daily_target_minutes, 468);
        let written = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert_eq!(written, DEFAULT_TOML);
        // second load reads the file
        std::fs::write(
            home.join("config.toml"),
            DEFAULT_TOML.replace("= 468", "= 480"),
        )
        .unwrap();
        assert_eq!(
            Config::load_or_create(&home).unwrap().daily_target_minutes,
            480
        );
    }

    #[test]
    fn extra_holidays_reach_calendar() {
        let toml = DEFAULT_TOML.replace("extra_holidays = []", "extra_holidays = [\"2026-12-24\"]");
        let c = Config::from_toml(&toml).unwrap();
        assert!(
            c.calendar()
                .is_holiday(NaiveDate::from_ymd_opt(2026, 12, 24).unwrap())
        );
    }

    #[test]
    fn resolve_home_prefers_flag_then_env() {
        let flag = std::path::Path::new("/tmp/x");
        assert_eq!(resolve_home(Some(flag)), flag.to_path_buf());
        // env is process-global; run this check in isolation
        unsafe { std::env::set_var("TK_HOME", "/tmp/envhome") };
        assert_eq!(resolve_home(None), std::path::PathBuf::from("/tmp/envhome"));
        unsafe { std::env::remove_var("TK_HOME") };
        assert!(resolve_home(None).ends_with("tk"));
    }
}
