use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use serde::Deserialize;
use thiserror::Error;
use toml_edit::{DocumentMut, Item, value};

use crate::core::{BreakTier, HolidayCalendar, Minutes, Rules};

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
extra_holidays = []                # e.g. ["2026-12-24", "2026-12-31"]

[[break_tiers]]                    # ascending; last matching tier applies
after_minutes = 180
deduct_minutes = 18

[[break_tiers]]
after_minutes = 360
deduct_minutes = 48

[theme_overrides]                  # optional; any role may be set to "#rrggbb" or a named color
# positive = "#a6e3a1"
"##;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
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
    #[serde(default)]
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
        let c: Config = toml::from_str(s).map_err(|e| ConfigError::Parse(e.to_string()))?;
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

/// The four settings `tk config` and the TUI settings overlay may change. `None`
/// leaves the key in the file exactly as it is.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ConfigPatch {
    pub start_date: Option<NaiveDate>,
    pub initial_balance_minutes: Option<i32>,
    pub daily_target_minutes: Option<i32>,
    pub vacation_days_per_year: Option<u32>,
}

/// Write `v` to a root key, keeping every byte of formatting around it.
///
/// A line's formatting lives in two places: the *key* carries what comes before
/// it — the comment lines and blank lines above it, and the file header if it is
/// the first key — and the *value* carries the padding and the trailing
/// `# comment` after it. `Table::insert` replaces the stored key, and with it the
/// comments above; re-inserting the existing key with `insert_formatted` keeps
/// them. A key that is not in the file yet is appended to the root table, which
/// `toml_edit` renders before the first `[[break_tiers]]`, because a table's own
/// key/values always come before its sub-tables.
fn set_root(doc: &mut DocumentMut, key: &str, v: Item) {
    let table = doc.as_table_mut();
    let old_key = table.key(key).cloned();
    let decor = table
        .get(key)
        .and_then(Item::as_value)
        .map(|v| v.decor().clone());
    match &old_key {
        Some(k) => table.insert_formatted(k, v),
        None => table.insert(key, v),
    };
    if let Some(d) = decor
        && let Some(nv) = table.get_mut(key).and_then(Item::as_value_mut)
    {
        *nv.decor_mut() = d;
    }
}

impl Config {
    /// Apply `patch` to `home/config.toml`, creating it from [`DEFAULT_TOML`] when
    /// absent, and return the reloaded config.
    ///
    /// The rewrite goes through `toml_edit`, so comments, key order and the
    /// `[[break_tiers]]` tables survive untouched. The result is parsed and
    /// validated *before* anything is written: an invalid patch leaves the file
    /// exactly as it was.
    pub fn write_updates(home: &Path, patch: &ConfigPatch) -> Result<Config, ConfigError> {
        std::fs::create_dir_all(home)?;
        let path = home.join("config.toml");
        if !path.exists() {
            std::fs::write(&path, DEFAULT_TOML)?;
        }
        let text = std::fs::read_to_string(&path)?;
        let mut doc: DocumentMut = text
            .parse()
            .map_err(|e: toml_edit::TomlError| ConfigError::Parse(e.to_string()))?;
        if let Some(d) = patch.start_date {
            set_root(&mut doc, "start_date", value(d.to_string()));
        }
        if let Some(b) = patch.initial_balance_minutes {
            set_root(&mut doc, "initial_balance_minutes", value(b as i64));
        }
        if let Some(t) = patch.daily_target_minutes {
            set_root(&mut doc, "daily_target_minutes", value(t as i64));
        }
        if let Some(v) = patch.vacation_days_per_year {
            set_root(&mut doc, "vacation_days_per_year", value(v as i64));
        }
        let new_text = doc.to_string();
        let cfg = Config::from_toml(&new_text)?;
        // Write beside the file and rename over it: a crash or a full disk leaves
        // either the old config or the new one, never half of either.
        let tmp = home.join("config.toml.tmp");
        std::fs::write(&tmp, &new_text)?;
        std::fs::rename(&tmp, &path)?;
        Ok(cfg)
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

    /// The environment is process-global, so any test that writes it holds this lock
    /// for its whole body. `cargo test` runs the suite in parallel threads.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Puts `TK_HOME` back exactly as it was, panic or not.
    struct TkHome(Option<String>);

    impl TkHome {
        fn set(v: &str) -> TkHome {
            let prev = std::env::var("TK_HOME").ok();
            unsafe { std::env::set_var("TK_HOME", v) };
            TkHome(prev)
        }
    }

    impl Drop for TkHome {
        fn drop(&mut self) {
            match self.0.take() {
                Some(v) => unsafe { std::env::set_var("TK_HOME", v) },
                None => unsafe { std::env::remove_var("TK_HOME") },
            }
        }
    }

    #[test]
    fn resolve_home_prefers_flag_then_env() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let flag = std::path::Path::new("/tmp/x");
        assert_eq!(resolve_home(Some(flag)), flag.to_path_buf());
        let restore = TkHome::set("/tmp/envhome");
        assert_eq!(resolve_home(None), std::path::PathBuf::from("/tmp/envhome"));
        // The flag still wins over a set variable.
        assert_eq!(resolve_home(Some(flag)), flag.to_path_buf());
        // An empty variable is ignored, as if it were unset.
        let _empty = TkHome::set("");
        assert!(resolve_home(None).ends_with("tk"));
        drop(_empty);
        drop(restore);
    }

    #[test]
    fn write_updates_keeps_comments_and_tables() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("h");
        let patch = ConfigPatch {
            // `start_date` is the key the file header sits above, and
            // `daily_target_minutes` carries a trailing comment.
            start_date: Some(NaiveDate::from_ymd_opt(2026, 4, 1).unwrap()),
            daily_target_minutes: Some(480),
            vacation_days_per_year: Some(28),
            ..Default::default()
        };
        // The file does not exist yet: it is created from the default first.
        let c = Config::write_updates(&home, &patch).unwrap();
        assert_eq!(c.daily_target_minutes, 480);
        assert_eq!(c.vacation_days_per_year, 28);
        let written = std::fs::read_to_string(home.join("config.toml")).unwrap();
        let expected = DEFAULT_TOML
            .replace("start_date = \"2026-01-01\"", "start_date = \"2026-04-01\"")
            .replace("daily_target_minutes = 468", "daily_target_minutes = 480")
            .replace("vacation_days_per_year = 30", "vacation_days_per_year = 28");
        // Byte-for-byte except the three changed values: the file header, the
        // comments on and above each key, the alignment and the `[[break_tiers]]`
        // tables all survive.
        assert_eq!(written, expected);
    }

    #[test]
    fn write_updates_keeps_the_comments_above_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("h");
        std::fs::create_dir_all(&home).unwrap();
        // A hand-edited file: a note and a blank line directly above the key, and a
        // note above a key the patch does not touch.
        let original = DEFAULT_TOML.replace(
            "daily_target_minutes = 468",
            "# agreed with HR on 2026-03-01\n\ndaily_target_minutes = 468",
        );
        std::fs::write(home.join("config.toml"), &original).unwrap();
        Config::write_updates(
            &home,
            &ConfigPatch {
                daily_target_minutes: Some(480),
                ..Default::default()
            },
        )
        .unwrap();
        let written = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert_eq!(
            written,
            original.replace("daily_target_minutes = 468", "daily_target_minutes = 480")
        );
    }

    #[test]
    fn write_updates_writes_dates_and_balances() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("h");
        let patch = ConfigPatch {
            start_date: Some(NaiveDate::from_ymd_opt(2026, 9, 15).unwrap()),
            initial_balance_minutes: Some(-90),
            ..Default::default()
        };
        let c = Config::write_updates(&home, &patch).unwrap();
        assert_eq!(c.start_date, NaiveDate::from_ymd_opt(2026, 9, 15).unwrap());
        assert_eq!(c.initial_balance_minutes, -90);
        let written = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(written.contains("start_date = \"2026-09-15\""), "{written}");
        assert!(
            written.contains("initial_balance_minutes = -90"),
            "{written}"
        );
        // The reloaded config is what a later run would see.
        assert_eq!(Config::load_or_create(&home).unwrap(), c);
    }

    #[test]
    fn write_updates_rejects_an_invalid_patch_without_touching_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("h");
        Config::load_or_create(&home).unwrap();
        let before = std::fs::read_to_string(home.join("config.toml")).unwrap();
        let patch = ConfigPatch {
            daily_target_minutes: Some(0),
            ..Default::default()
        };
        match Config::write_updates(&home, &patch) {
            Err(ConfigError::Validation { field, .. }) => assert_eq!(field, "daily_target_minutes"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            std::fs::read_to_string(home.join("config.toml")).unwrap(),
            before
        );
    }

    #[test]
    fn write_updates_reinserts_a_missing_key_at_root() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("h");
        std::fs::create_dir_all(&home).unwrap();
        let stripped: String = DEFAULT_TOML
            .lines()
            .filter(|l| !l.starts_with("vacation_days_per_year"))
            .map(|l| format!("{l}\n"))
            .collect();
        std::fs::write(home.join("config.toml"), &stripped).unwrap();
        let c = Config::write_updates(
            &home,
            &ConfigPatch {
                vacation_days_per_year: Some(25),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(c.vacation_days_per_year, 25);
        let written = std::fs::read_to_string(home.join("config.toml")).unwrap();
        let key = written
            .find("vacation_days_per_year = 25")
            .unwrap_or_else(|| panic!("{written}"));
        let tier = written.find("[[break_tiers]]").unwrap();
        assert!(key < tier, "the key must land at root level:\n{written}");
        // And the file still parses as a whole.
        Config::from_toml(&written).unwrap();
    }
}
