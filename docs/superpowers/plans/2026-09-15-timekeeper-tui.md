# Timekeeper (`tk`) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `tk`, a Rust flexitime tracker with a full-screen tui-realm TUI plus quick CLI commands, SQLite storage in one portable folder, and Saxony (Dresden) holidays.

**Architecture:** A single crate with a pure `core` module (balance math, breaks, holidays, parsing) that has no I/O; a `store` module that is the only SQLite user; `config` for the TOML file; `cli` for clap commands; and `tui` built on tui-realm 4.1 with tokio async ports for a 1 s ticker and a store worker. Every screen's drawing is a pure function over view structs so it can be snapshot-tested with ratatui's `TestBackend`.

**Tech Stack:** Rust 2024 (rustc 1.97 via nix), tuirealm 4.1 (+ `async-ports`), tui-realm-stdlib 4.1, ratatui 0.30, crossterm 0.29, tokio 1, clap 4 (derive), rusqlite 0.40 (`bundled`), serde + toml, chrono 0.4, thiserror, anyhow, directories, tempfile, assert_cmd.

**Spec:** `docs/superpowers/specs/2026-09-15-timekeeper-tui-design.md` — read it first; this plan argues from it.

## Global Constraints

- Binary name is `tk`; repository is `~/timekeeper`.
- `core` has no I/O and does not depend on `store`, `config`, or `tui`. `store` is the only module importing `rusqlite`. `tui` and `cli` never depend on each other.
- Holidays: Saxony as applicable in Dresden, exactly the 11 days in spec §5.2; Fronleichnam is **not** included.
- Break deduction: last tier whose `after_minutes < gross` applies (strictly less); applied once per day on daily gross.
- Target only for Mon–Fri with kind `Work` or `Flex`. Today's target counts only when not clocked in and at least one entry exists.
- Home directory resolution: `--home` flag, then `TK_HOME`, then `$XDG_DATA_HOME/tk` (default `~/.local/share/tk`).
- Config file defaults exactly as spec §6.3 (`daily_target_minutes = 468`, tiers 180→18 and 360→48).
- Durations always render `+HH:MM` / `-HH:MM`.
- Minimum terminal 80×24; comment column hides below 90 columns; summary box stacks below 100.
- Keys (month view): `↑↓`/`jk` day, `[`/`]` month, `t` today, `Enter` day editor, `s` stats, `i`/`o` clock in/out, `v` `f` `x` `p` set vacation/flex/sick/holiday, `w` reset to work, `?` help, `q` quit, `Esc` back.
- Toolchain: there is no cargo on PATH. Every cargo command in this plan is run as `nix develop -c cargo ...` from the repo root (Task 1 creates the flake). Until Task 1's flake exists use `nix shell nixpkgs#cargo nixpkgs#rustc --command cargo ...`.
- Commits end with:
  ```
  Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01EjHrRPeVtsE6YmCsRCJ9FF
  ```
- `cargo fmt` and `cargo clippy -- -D warnings` must pass before each commit.

## File map

```
Cargo.toml, flake.nix, .gitignore, README.md, rustfmt.toml
src/main.rs                 parse CLI, dispatch to cli::run or tui::run
src/lib.rs                  pub mod core; pub mod store; pub mod config; pub mod cli; pub mod tui;
src/core/mod.rs             re-exports
src/core/error.rs           CoreError
src/core/minutes.rs         Minutes newtype
src/core/types.rs           DayKind, Entry, Day, Project
src/core/time_parse.rs      parse_time, parse_date
src/core/breaks.rs          BreakTier, deduction
src/core/holidays.rs        easter_sunday, saxony_holidays, HolidayCalendar
src/core/balance.rs         Rules, TodayCtx, DayStats, effective_kind, day_stats, running_balance, check_overlap
src/config/mod.rs           Config, ConfigError, load_or_create, resolve_home
src/store/mod.rs            Store, StoreError, open, migrations
src/store/schema.sql
src/store/projects.rs
src/store/days.rs
src/store/entries.rs
src/store/session.rs
src/cli/mod.rs              clap types
src/cli/commands.rs         command implementations
src/tui/mod.rs              run(): runtime, Application, main loop
src/tui/ids.rs              Id
src/tui/msg.rs              Msg, UserEvent, StoreCmd, StoreReply
src/tui/theme.rs            Theme
src/tui/worker.rs           store worker thread + StorePort (PollAsync)
src/tui/model.rs            Model, Screen, update()
src/tui/view/mod.rs         shared draw helpers (chips, minutes spans, bars)
src/tui/view/month.rs       MonthView + build_month_view + draw_month_table + draw_summary
src/tui/view/day.rs         DayView + draw_day_editor
src/tui/view/stats.rs       StatsView + build_stats + draw_stats
src/tui/view/chrome.rs      draw_title_bar, draw_status_bar, draw_key_hints, draw_help, draw_confirm, draw_too_small
src/tui/components/mod.rs   tui-realm components (thin wrappers that forward keys → Msg)
src/tui/components/month.rs
src/tui/components/day.rs
src/tui/components/form.rs  EntryForm (4 fields, live footer, validation)
src/tui/components/stats.rs
src/tui/components/bridge.rs Phantom component subscribed to Tick + User events
tests/cli.rs                assert_cmd tests
tests/fixtures/            holiday fixtures
```

---

### Task 1: Crate skeleton, flake, and CI script

**Files:**
- Create: `Cargo.toml`, `flake.nix`, `.gitignore`, `rustfmt.toml`, `src/main.rs`, `src/lib.rs`, `src/core/mod.rs`, `src/store/mod.rs`, `src/config/mod.rs`, `src/cli/mod.rs`, `src/tui/mod.rs`, `scripts/check.sh`

**Interfaces:**
- Produces: a compiling crate `tk` where `cargo test` runs zero tests successfully; `nix develop -c cargo build` works.

- [ ] **Step 1: Write Cargo.toml**

```toml
[package]
name = "tk"
version = "0.1.0"
edition = "2024"
rust-version = "1.88"
description = "Flexitime (Gleitzeit) tracker with a terminal UI"
license = "MIT"

[[bin]]
name = "tk"
path = "src/main.rs"

[lib]
name = "tk"
path = "src/lib.rs"

[dependencies]
anyhow = "1"
chrono = { version = "0.4", features = ["serde"] }
clap = { version = "4", features = ["derive"] }
directories = "6"
rusqlite = { version = "0.40", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
thiserror = "2"
tokio = { version = "1", features = ["rt-multi-thread", "sync", "macros", "time"] }
toml = "0.9"
tuirealm = { version = "4.1", features = ["async-ports"] }
tui-realm-stdlib = "4.1"

[dev-dependencies]
assert_cmd = "2"
predicates = "3"
tempfile = "3"

[profile.release]
lto = true
codegen-units = 1
strip = true
```

If `directories = "6"` or `toml = "0.9"` does not resolve, run `nix shell nixpkgs#cargo --command cargo search <crate> --limit 1` and pin the latest major.

- [ ] **Step 2: Write flake.nix**

```nix
{
  description = "tk - flexitime tracker TUI";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };
  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let pkgs = import nixpkgs { inherit system; };
      in {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "tk";
          version = "0.1.0";
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.sqlite ];
        };
        devShells.default = pkgs.mkShell {
          packages = with pkgs; [ cargo rustc rustfmt clippy rust-analyzer sqlite pkg-config ];
          RUST_SRC_PATH = "${pkgs.rust.packages.stable.rustPlatform.rustLibSrc}";
        };
      });
}
```

- [ ] **Step 3: Write .gitignore, rustfmt.toml, scripts/check.sh**

`.gitignore`:
```
/target
/result
*.db
*.db-wal
*.db-shm
```

`rustfmt.toml`:
```toml
edition = "2024"
```

`scripts/check.sh` (make executable with `chmod +x`):
```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
nix develop -c cargo fmt --check
nix develop -c cargo clippy --all-targets -- -D warnings
nix develop -c cargo test
```

- [ ] **Step 4: Write module stubs**

`src/lib.rs`:
```rust
pub mod cli;
pub mod config;
pub mod core;
pub mod store;
pub mod tui;
```

`src/main.rs`:
```rust
fn main() -> anyhow::Result<()> {
    println!("tk 0.1.0");
    Ok(())
}
```

`src/core/mod.rs`, `src/store/mod.rs`, `src/config/mod.rs`, `src/cli/mod.rs`, `src/tui/mod.rs`: each contains only `//! module` for now.

- [ ] **Step 5: Build and run**

Run: `nix develop -c cargo build && nix develop -c cargo run`
Expected: prints `tk 0.1.0`. First build downloads crates and compiles SQLite; allow several minutes. If `nix develop` fails on `RUST_SRC_PATH`, delete that line.

- [ ] **Step 6: Check and commit**

Run: `./scripts/check.sh`
Expected: all three pass (zero tests).

```bash
git add -A
git commit -m "chore: crate skeleton, nix flake, check script"
```

---

### Task 2: `Minutes` newtype and `CoreError`

**Files:**
- Create: `src/core/error.rs`, `src/core/minutes.rs`
- Modify: `src/core/mod.rs`

**Interfaces:**
- Produces:
  - `pub struct Minutes(pub i32)` with `ZERO`, `hm(h,m)`, `is_negative()`, `abs()`, `Add`, `Sub`, `Neg`, `AddAssign`, `Sum`, `Display` (`+HH:MM`/`-HH:MM`), `FromStr` (`[+-]HH:MM`), `Ord`, `Default`, `Copy`, `Hash`.
  - `pub enum CoreError { Overlap, InvalidTime(String), InvalidDate(String), InvalidRange }`.

- [ ] **Step 1: Write failing tests in `src/core/minutes.rs`**

```rust
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
}
```

- [ ] **Step 2: Run to verify failure**

Run: `nix develop -c cargo test minutes`
Expected: compile error, `Minutes` not defined.

- [ ] **Step 3: Implement**

`src/core/error.rs`:
```rust
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum CoreError {
    #[error("entry overlaps an existing entry")]
    Overlap,
    #[error("invalid time: {0}")]
    InvalidTime(String),
    #[error("invalid date: {0}")]
    InvalidDate(String),
    #[error("end must be after start")]
    InvalidRange,
}
```

`src/core/minutes.rs`:
```rust
use std::fmt;
use std::iter::Sum;
use std::ops::{Add, AddAssign, Neg, Sub};
use std::str::FromStr;

use super::error::CoreError;

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
        let h: i32 = h.parse().map_err(|_| CoreError::InvalidTime(s.to_string()))?;
        let m: i32 = m.parse().map_err(|_| CoreError::InvalidTime(s.to_string()))?;
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
```

`src/core/mod.rs`:
```rust
pub mod error;
pub mod minutes;

pub use error::CoreError;
pub use minutes::Minutes;
```

- [ ] **Step 4: Run tests**

Run: `nix develop -c cargo test minutes`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src/core
git commit -m "feat(core): Minutes newtype and CoreError"
```

---

### Task 3: Domain types (`DayKind`, `Entry`, `Day`, `Project`)

**Files:**
- Create: `src/core/types.rs`
- Modify: `src/core/mod.rs`

**Interfaces:**
- Produces:
  - `pub enum DayKind { Work, Vacation, Flex, Holiday, Sick, Absence { label: String } }` with `as_str()`, `parse(kind: &str, label: Option<&str>) -> Option<DayKind>`, `has_target() -> bool`, `display_name() -> String`, `allows_entries() -> bool`.
  - `pub struct Entry { pub id: i64, pub date: NaiveDate, pub start: NaiveTime, pub end: NaiveTime, pub project: String, pub comment: String }` with `duration() -> Minutes`, `interval() -> (i32, i32)` (minutes since midnight; end > 1440 when crossing midnight), `crosses_midnight() -> bool`.
  - `pub struct Day { pub date: NaiveDate, pub kind: DayKind, pub entries: Vec<Entry> }`.
  - `pub struct Project { pub id: i64, pub name: String, pub color_index: u8, pub archived: bool }`.

- [ ] **Step 1: Write failing tests (bottom of `src/core/types.rs`)**

```rust
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
        assert_eq!(a, DayKind::Absence { label: "training".into() });
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
```

- [ ] **Step 2: Run to verify failure**

Run: `nix develop -c cargo test types`
Expected: compile error.

- [ ] **Step 3: Implement `src/core/types.rs`**

```rust
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
```

Add to `src/core/mod.rs`: `pub mod types;` and `pub use types::{Day, DayKind, Entry, Project, minutes_of};`.

- [ ] **Step 4: Run tests**

Run: `nix develop -c cargo test types`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src/core
git commit -m "feat(core): DayKind, Entry, Day, Project types"
```

---

### Task 4: Time and date parsing

**Files:**
- Create: `src/core/time_parse.rs`
- Modify: `src/core/mod.rs`

**Interfaces:**
- Produces:
  - `pub fn parse_time(s: &str) -> Result<NaiveTime, CoreError>` accepting `8`, `800`, `0800`, `8:00`, `17:30`, `1730`, `17.30`.
  - `pub fn parse_date(s: &str, today: NaiveDate) -> Result<NaiveDate, CoreError>` accepting `YYYY-MM-DD`, `today`, `yesterday`, `tomorrow`, signed offsets like `-1` / `+2`.
  - `pub fn parse_time_range(s: &str) -> Result<(NaiveTime, NaiveTime), CoreError>` for `0900-1530`.

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{NaiveDate, NaiveTime};

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
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
        assert_eq!(parse_date("yesterday", today).unwrap(), today.pred_opt().unwrap());
        assert_eq!(parse_date("tomorrow", today).unwrap(), today.succ_opt().unwrap());
        assert_eq!(parse_date("-1", today).unwrap(), today.pred_opt().unwrap());
        assert_eq!(parse_date("+2", today).unwrap(), NaiveDate::from_ymd_opt(2026, 9, 17).unwrap());
        assert_eq!(parse_date("0", today).unwrap(), today);
        assert_eq!(parse_date("2026-01-31", today).unwrap(), NaiveDate::from_ymd_opt(2026, 1, 31).unwrap());
        assert!(parse_date("2026-02-30", today).is_err());
        assert!(parse_date("nope", today).is_err());
    }

    #[test]
    fn parses_range() {
        assert_eq!(parse_time_range("0900-1530").unwrap(), (t(9, 0), t(15, 30)));
        assert_eq!(parse_time_range("9:00-17:30").unwrap(), (t(9, 0), t(17, 30)));
        assert!(parse_time_range("0900").is_err());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `nix develop -c cargo test time_parse`
Expected: compile error.

- [ ] **Step 3: Implement `src/core/time_parse.rs`**

```rust
use chrono::{Days, NaiveDate, NaiveTime};

use super::CoreError;

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

pub fn parse_time_range(s: &str) -> Result<(NaiveTime, NaiveTime), CoreError> {
    let (a, b) = s
        .split_once('-')
        .ok_or_else(|| CoreError::InvalidTime(s.to_string()))?;
    Ok((parse_time(a)?, parse_time(b)?))
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
            today.checked_add_days(Days::new(off as u64)).ok_or_else(err)
        } else {
            today.checked_sub_days(Days::new(off.unsigned_abs())).ok_or_else(err)
        };
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| err())
}
```

Add to `src/core/mod.rs`: `pub mod time_parse;` and `pub use time_parse::{parse_date, parse_time, parse_time_range};`.

- [ ] **Step 4: Run tests**

Run: `nix develop -c cargo test time_parse`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src/core
git commit -m "feat(core): time and date parsing"
```

---

### Task 5: Break tiers

**Files:**
- Create: `src/core/breaks.rs`
- Modify: `src/core/mod.rs`

**Interfaces:**
- Produces: `pub struct BreakTier { pub after: Minutes, pub deduct: Minutes }` (Clone, Debug, PartialEq), `pub fn deduction(gross: Minutes, tiers: &[BreakTier]) -> Minutes`, `pub fn default_tiers() -> Vec<BreakTier>` (180→18, 360→48).

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundaries_are_strict() {
        let t = default_tiers();
        assert_eq!(deduction(Minutes(0), &t), Minutes(0));
        assert_eq!(deduction(Minutes(180), &t), Minutes(0)); // exactly 3h: not over
        assert_eq!(deduction(Minutes(181), &t), Minutes(18));
        assert_eq!(deduction(Minutes(360), &t), Minutes(18)); // exactly 6h: still tier 1
        assert_eq!(deduction(Minutes(361), &t), Minutes(48));
        assert_eq!(deduction(Minutes(600), &t), Minutes(48));
    }

    #[test]
    fn no_tiers_no_deduction() {
        assert_eq!(deduction(Minutes(999), &[]), Minutes(0));
    }

    #[test]
    fn unsorted_tiers_still_pick_highest_matching() {
        let t = vec![
            BreakTier { after: Minutes(360), deduct: Minutes(48) },
            BreakTier { after: Minutes(180), deduct: Minutes(18) },
        ];
        assert_eq!(deduction(Minutes(400), &t), Minutes(48));
        assert_eq!(deduction(Minutes(200), &t), Minutes(18));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `nix develop -c cargo test breaks`
Expected: compile error.

- [ ] **Step 3: Implement `src/core/breaks.rs`**

```rust
use super::Minutes;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BreakTier {
    pub after: Minutes,
    pub deduct: Minutes,
}

pub fn default_tiers() -> Vec<BreakTier> {
    vec![
        BreakTier { after: Minutes(180), deduct: Minutes(18) },
        BreakTier { after: Minutes(360), deduct: Minutes(48) },
    ]
}

/// Deduction of the tier with the largest `after` that is strictly below `gross`.
pub fn deduction(gross: Minutes, tiers: &[BreakTier]) -> Minutes {
    tiers
        .iter()
        .filter(|t| t.after < gross)
        .max_by_key(|t| t.after)
        .map(|t| t.deduct)
        .unwrap_or(Minutes::ZERO)
}
```

Add to `src/core/mod.rs`: `pub mod breaks;` and `pub use breaks::{BreakTier, deduction, default_tiers};`.

- [ ] **Step 4: Run tests**

Run: `nix develop -c cargo test breaks`
Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add src/core
git commit -m "feat(core): tiered break deduction"
```

---

### Task 6: Saxony (Dresden) holidays

**Files:**
- Create: `src/core/holidays.rs`, `tests/fixtures/saxony_holidays.txt`
- Modify: `src/core/mod.rs`

**Interfaces:**
- Produces:
  - `pub fn easter_sunday(year: i32) -> NaiveDate`
  - `pub fn buss_und_bettag(year: i32) -> NaiveDate`
  - `pub fn saxony_holidays(year: i32) -> Vec<(NaiveDate, &'static str)>` sorted by date, 11 entries.
  - `pub struct HolidayCalendar { extra: Vec<NaiveDate> }` with `new(extra: Vec<NaiveDate>)`, `holiday_name(&self, d: NaiveDate) -> Option<String>` (`"Extra holiday"` for extras), `is_holiday(&self, d) -> bool`.

- [ ] **Step 1: Write fixture `tests/fixtures/saxony_holidays.txt`**

One `YYYY-MM-DD name` per line. These are the legally correct dates; do not alter.

```
2024-01-01 Neujahr
2024-03-29 Karfreitag
2024-04-01 Ostermontag
2024-05-01 Tag der Arbeit
2024-05-09 Christi Himmelfahrt
2024-05-20 Pfingstmontag
2024-10-03 Tag der Deutschen Einheit
2024-10-31 Reformationstag
2024-11-20 Buß- und Bettag
2024-12-25 1. Weihnachtstag
2024-12-26 2. Weihnachtstag
2025-01-01 Neujahr
2025-04-18 Karfreitag
2025-04-21 Ostermontag
2025-05-01 Tag der Arbeit
2025-05-29 Christi Himmelfahrt
2025-06-09 Pfingstmontag
2025-10-03 Tag der Deutschen Einheit
2025-10-31 Reformationstag
2025-11-19 Buß- und Bettag
2025-12-25 1. Weihnachtstag
2025-12-26 2. Weihnachtstag
2026-01-01 Neujahr
2026-04-03 Karfreitag
2026-04-06 Ostermontag
2026-05-01 Tag der Arbeit
2026-05-14 Christi Himmelfahrt
2026-05-25 Pfingstmontag
2026-10-03 Tag der Deutschen Einheit
2026-10-31 Reformationstag
2026-11-18 Buß- und Bettag
2026-12-25 1. Weihnachtstag
2026-12-26 2. Weihnachtstag
2027-01-01 Neujahr
2027-03-26 Karfreitag
2027-03-29 Ostermontag
2027-05-01 Tag der Arbeit
2027-05-06 Christi Himmelfahrt
2027-05-17 Pfingstmontag
2027-10-03 Tag der Deutschen Einheit
2027-10-31 Reformationstag
2027-11-17 Buß- und Bettag
2027-12-25 1. Weihnachtstag
2027-12-26 2. Weihnachtstag
2028-01-01 Neujahr
2028-04-14 Karfreitag
2028-04-17 Ostermontag
2028-05-01 Tag der Arbeit
2028-05-25 Christi Himmelfahrt
2028-06-05 Pfingstmontag
2028-10-03 Tag der Deutschen Einheit
2028-10-31 Reformationstag
2028-11-22 Buß- und Bettag
2028-12-25 1. Weihnachtstag
2028-12-26 2. Weihnachtstag
2029-01-01 Neujahr
2029-03-30 Karfreitag
2029-04-02 Ostermontag
2029-05-01 Tag der Arbeit
2029-05-10 Christi Himmelfahrt
2029-05-21 Pfingstmontag
2029-10-03 Tag der Deutschen Einheit
2029-10-31 Reformationstag
2029-11-21 Buß- und Bettag
2029-12-25 1. Weihnachtstag
2029-12-26 2. Weihnachtstag
2030-01-01 Neujahr
2030-04-19 Karfreitag
2030-04-22 Ostermontag
2030-05-01 Tag der Arbeit
2030-05-30 Christi Himmelfahrt
2030-06-10 Pfingstmontag
2030-10-03 Tag der Deutschen Einheit
2030-10-31 Reformationstag
2030-11-20 Buß- und Bettag
2030-12-25 1. Weihnachtstag
2030-12-26 2. Weihnachtstag
```

- [ ] **Step 2: Write failing tests (bottom of `src/core/holidays.rs`)**

```rust
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
                (NaiveDate::parse_from_str(d, "%Y-%m-%d").unwrap(), n.to_string())
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
            cal.holiday_name(NaiveDate::from_ymd_opt(2026, 11, 18).unwrap()).as_deref(),
            Some("Buß- und Bettag")
        );
        assert!(!cal.is_holiday(NaiveDate::from_ymd_opt(2026, 9, 15).unwrap()));
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `nix develop -c cargo test holidays`
Expected: compile error.

- [ ] **Step 4: Implement `src/core/holidays.rs`**

```rust
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
```

Add to `src/core/mod.rs`: `pub mod holidays;` and `pub use holidays::{HolidayCalendar, easter_sunday, saxony_holidays};`.

- [ ] **Step 5: Run tests**

Run: `nix develop -c cargo test holidays`
Expected: 4 passed. If `matches_fixture` fails, the bug is in the code, not the fixture.

- [ ] **Step 6: Commit**

```bash
git add src/core tests/fixtures
git commit -m "feat(core): Saxony (Dresden) holiday calendar"
```

---

### Task 7: Balance engine

**Files:**
- Create: `src/core/balance.rs`
- Modify: `src/core/mod.rs`

**Interfaces:**
- Consumes: `Minutes`, `DayKind`, `Day`, `Entry`, `BreakTier`, `deduction`, `HolidayCalendar`, `CoreError`.
- Produces:
  ```rust
  pub struct Rules { pub daily_target: Minutes, pub tiers: Vec<BreakTier>, pub start_date: NaiveDate, pub initial_balance: Minutes }
  pub struct TodayCtx { pub today: NaiveDate, pub clocked_in: bool }
  pub struct DayStats { pub date: NaiveDate, pub kind: DayKind, pub gross: Minutes, pub deduction: Minutes, pub net: Minutes, pub target: Minutes, pub balance: Minutes, pub missing: bool, pub holiday_name: Option<String>, pub is_weekend: bool }
  pub fn is_working_day(d: NaiveDate) -> bool
  pub fn effective_kind(stored: Option<&DayKind>, date: NaiveDate, cal: &HolidayCalendar) -> DayKind
  pub fn day_stats(day: &Day, rules: &Rules, cal: &HolidayCalendar, ctx: &TodayCtx) -> DayStats
  pub fn running_balance<'a>(stats: impl IntoIterator<Item = &'a DayStats>, rules: &Rules) -> Minutes
  pub fn check_overlap(existing: &[Entry], start: NaiveTime, end: NaiveTime, ignore_id: Option<i64>) -> Result<(), CoreError>
  pub fn provisional_net(entries: &[Entry], running_since: NaiveTime, now: NaiveTime, rules: &Rules) -> Minutes
  ```

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::default_tiers;
    use chrono::{NaiveDate, NaiveTime};

    fn d(y: i32, m: u32, dd: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, dd).unwrap()
    }
    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }
    fn rules() -> Rules {
        Rules {
            daily_target: Minutes(468),
            tiers: default_tiers(),
            start_date: d(2026, 9, 1),
            initial_balance: Minutes(60),
        }
    }
    fn cal() -> HolidayCalendar {
        HolidayCalendar::new(vec![])
    }
    fn entry(id: i64, date: NaiveDate, s: NaiveTime, e: NaiveTime) -> Entry {
        Entry { id, date, start: s, end: e, project: "Alpha".into(), comment: String::new() }
    }
    fn work(date: NaiveDate, entries: Vec<Entry>) -> Day {
        Day { date, kind: DayKind::Work, entries }
    }
    fn ctx(today: NaiveDate, clocked_in: bool) -> TodayCtx {
        TodayCtx { today, clocked_in }
    }

    #[test]
    fn working_days_are_mon_to_fri() {
        assert!(is_working_day(d(2026, 9, 14))); // Mon
        assert!(is_working_day(d(2026, 9, 18))); // Fri
        assert!(!is_working_day(d(2026, 9, 19))); // Sat
        assert!(!is_working_day(d(2026, 9, 20))); // Sun
    }

    #[test]
    fn effective_kind_prefers_stored_then_holiday_then_work() {
        let c = cal();
        let xmas = d(2026, 12, 25);
        assert_eq!(effective_kind(None, xmas, &c), DayKind::Holiday);
        assert_eq!(effective_kind(Some(&DayKind::Work), xmas, &c), DayKind::Work);
        assert_eq!(effective_kind(None, d(2026, 9, 15), &c), DayKind::Work);
    }

    #[test]
    fn full_past_work_day() {
        let day = work(d(2026, 9, 14), vec![entry(1, d(2026, 9, 14), t(8, 0), t(17, 0))]);
        let s = day_stats(&day, &rules(), &cal(), &ctx(d(2026, 9, 15), false));
        assert_eq!(s.gross, Minutes(540));
        assert_eq!(s.deduction, Minutes(48));
        assert_eq!(s.net, Minutes(492));
        assert_eq!(s.target, Minutes(468));
        assert_eq!(s.balance, Minutes(24));
        assert!(!s.missing);
    }

    #[test]
    fn deduction_is_per_day_not_per_entry() {
        let day = work(
            d(2026, 9, 14),
            vec![
                entry(1, d(2026, 9, 14), t(8, 0), t(11, 0)),
                entry(2, d(2026, 9, 14), t(12, 0), t(15, 0)),
            ],
        );
        let s = day_stats(&day, &rules(), &cal(), &ctx(d(2026, 9, 15), false));
        assert_eq!(s.gross, Minutes(360));
        assert_eq!(s.deduction, Minutes(18));
    }

    #[test]
    fn past_weekday_without_entries_is_missing() {
        let s = day_stats(&work(d(2026, 9, 14), vec![]), &rules(), &cal(), &ctx(d(2026, 9, 15), false));
        assert!(s.missing);
        assert_eq!(s.balance, Minutes(-468));
    }

    #[test]
    fn weekend_has_no_target_and_is_not_missing() {
        let s = day_stats(&work(d(2026, 9, 12), vec![]), &rules(), &cal(), &ctx(d(2026, 9, 15), false));
        assert!(s.is_weekend);
        assert!(!s.missing);
        assert_eq!(s.balance, Minutes(0));
        let s2 = day_stats(
            &work(d(2026, 9, 12), vec![entry(1, d(2026, 9, 12), t(10, 0), t(12, 0))]),
            &rules(), &cal(), &ctx(d(2026, 9, 15), false),
        );
        assert_eq!(s2.balance, Minutes(120));
    }

    #[test]
    fn kinds() {
        let r = rules();
        let c = cal();
        let cx = ctx(d(2026, 9, 15), false);
        let mk = |kind| Day { date: d(2026, 9, 14), kind, entries: vec![] };
        assert_eq!(day_stats(&mk(DayKind::Vacation), &r, &c, &cx).balance, Minutes(0));
        assert_eq!(day_stats(&mk(DayKind::Sick), &r, &c, &cx).balance, Minutes(0));
        assert_eq!(day_stats(&mk(DayKind::Absence { label: "trip".into() }), &r, &c, &cx).balance, Minutes(0));
        assert_eq!(day_stats(&mk(DayKind::Flex), &r, &c, &cx).balance, Minutes(-468));
        let hol = Day { date: d(2026, 12, 25), kind: DayKind::Holiday, entries: vec![] };
        let hs = day_stats(&hol, &r, &c, &cx);
        assert_eq!(hs.balance, Minutes(0));
        assert_eq!(hs.holiday_name.as_deref(), Some("1. Weihnachtstag"));
        assert!(!day_stats(&mk(DayKind::Vacation), &r, &c, &cx).missing);
    }

    #[test]
    fn today_rules() {
        let today = d(2026, 9, 15);
        let r = rules();
        let c = cal();
        // no entries, not clocked in: no target, not missing
        let s = day_stats(&work(today, vec![]), &r, &c, &ctx(today, false));
        assert_eq!(s.target, Minutes(0));
        assert!(!s.missing);
        // one entry, not clocked in: target applies
        let s = day_stats(&work(today, vec![entry(1, today, t(8, 0), t(12, 0))]), &r, &c, &ctx(today, false));
        assert_eq!(s.target, Minutes(468));
        assert_eq!(s.balance, Minutes(240 - 18 - 468));
        // one entry, clocked in: target withheld
        let s = day_stats(&work(today, vec![entry(1, today, t(8, 0), t(12, 0))]), &r, &c, &ctx(today, true));
        assert_eq!(s.target, Minutes(0));
        // future day: nothing
        let s = day_stats(&work(d(2026, 9, 16), vec![]), &r, &c, &ctx(today, false));
        assert_eq!(s.target, Minutes(0));
        assert!(!s.missing);
    }

    #[test]
    fn running_balance_sums_from_initial() {
        let r = rules();
        let c = cal();
        let cx = ctx(d(2026, 9, 16), false);
        let days = [
            work(d(2026, 9, 14), vec![entry(1, d(2026, 9, 14), t(8, 0), t(17, 0))]), // +24
            work(d(2026, 9, 15), vec![]),                                            // -468
        ];
        let stats: Vec<DayStats> = days.iter().map(|dd| day_stats(dd, &r, &c, &cx)).collect();
        assert_eq!(running_balance(&stats, &r), Minutes(60 + 24 - 468));
    }

    #[test]
    fn overlap_detection() {
        let ex = vec![
            entry(1, d(2026, 9, 14), t(9, 0), t(12, 0)),
            entry(2, d(2026, 9, 14), t(22, 0), t(2, 0)), // crosses midnight
        ];
        assert!(check_overlap(&ex, t(12, 0), t(13, 0), None).is_ok()); // touching is fine
        assert_eq!(check_overlap(&ex, t(11, 0), t(13, 0), None), Err(CoreError::Overlap));
        assert_eq!(check_overlap(&ex, t(23, 0), t(23, 30), None), Err(CoreError::Overlap));
        assert!(check_overlap(&ex, t(11, 0), t(13, 0), Some(1)).is_ok()); // editing entry 1
        assert_eq!(check_overlap(&ex, t(12, 0), t(12, 0), None), Err(CoreError::Overlap)); // 24h span
    }

    #[test]
    fn provisional_net_includes_running_entry() {
        let today = d(2026, 9, 15);
        let ex = vec![entry(1, today, t(8, 0), t(12, 0))]; // 240
        // running since 13:00, now 15:30 → +150 → gross 390 → deduct 48 → 342
        assert_eq!(provisional_net(&ex, t(13, 0), t(15, 30), &rules()), Minutes(342));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `nix develop -c cargo test balance`
Expected: compile error.

- [ ] **Step 3: Implement `src/core/balance.rs`**

```rust
use chrono::{Datelike, NaiveDate, NaiveTime, Weekday};

use super::{BreakTier, CoreError, Day, DayKind, Entry, HolidayCalendar, Minutes, deduction, minutes_of};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rules {
    pub daily_target: Minutes,
    pub tiers: Vec<BreakTier>,
    pub start_date: NaiveDate,
    pub initial_balance: Minutes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TodayCtx {
    pub today: NaiveDate,
    pub clocked_in: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayStats {
    pub date: NaiveDate,
    pub kind: DayKind,
    pub gross: Minutes,
    pub deduction: Minutes,
    pub net: Minutes,
    pub target: Minutes,
    pub balance: Minutes,
    pub missing: bool,
    pub holiday_name: Option<String>,
    pub is_weekend: bool,
}

pub fn is_working_day(d: NaiveDate) -> bool {
    !matches!(d.weekday(), Weekday::Sat | Weekday::Sun)
}

pub fn effective_kind(stored: Option<&DayKind>, date: NaiveDate, cal: &HolidayCalendar) -> DayKind {
    if let Some(k) = stored {
        return k.clone();
    }
    if cal.is_holiday(date) {
        DayKind::Holiday
    } else {
        DayKind::Work
    }
}

pub fn day_stats(day: &Day, rules: &Rules, cal: &HolidayCalendar, ctx: &TodayCtx) -> DayStats {
    let gross: Minutes = day.entries.iter().map(Entry::duration).sum();
    let ded = if gross > Minutes::ZERO { deduction(gross, &rules.tiers) } else { Minutes::ZERO };
    let net = Minutes((gross - ded).0.max(0));
    let is_weekend = !is_working_day(day.date);
    let is_past = day.date < ctx.today;
    let is_today = day.date == ctx.today;

    let target_applies = !is_weekend
        && day.kind.has_target()
        && (is_past || (is_today && !ctx.clocked_in && !day.entries.is_empty()));
    let target = if target_applies { rules.daily_target } else { Minutes::ZERO };

    let missing = is_past && !is_weekend && day.kind == DayKind::Work && day.entries.is_empty();

    DayStats {
        date: day.date,
        kind: day.kind.clone(),
        gross,
        deduction: ded,
        net,
        target,
        balance: net - target,
        missing,
        holiday_name: cal.holiday_name(day.date),
        is_weekend,
    }
}

pub fn running_balance<'a>(stats: impl IntoIterator<Item = &'a DayStats>, rules: &Rules) -> Minutes {
    rules.initial_balance
        + stats
            .into_iter()
            .filter(|s| s.date >= rules.start_date)
            .map(|s| s.balance)
            .sum::<Minutes>()
}

fn normalized(start: NaiveTime, end: NaiveTime) -> (i32, i32) {
    let s = minutes_of(start);
    let mut e = minutes_of(end);
    if e <= s {
        e += 1440;
    }
    (s, e)
}

pub fn check_overlap(
    existing: &[Entry],
    start: NaiveTime,
    end: NaiveTime,
    ignore_id: Option<i64>,
) -> Result<(), CoreError> {
    let (ns, ne) = normalized(start, end);
    for e in existing.iter().filter(|e| Some(e.id) != ignore_id) {
        let (os, oe) = e.interval();
        if ns < oe && ne > os {
            return Err(CoreError::Overlap);
        }
    }
    Ok(())
}

/// Net for today if the running clock-in were closed right now.
pub fn provisional_net(entries: &[Entry], running_since: NaiveTime, now: NaiveTime, rules: &Rules) -> Minutes {
    let (s, e) = normalized(running_since, now);
    let running = if now == running_since { Minutes::ZERO } else { Minutes(e - s) };
    let gross = entries.iter().map(Entry::duration).sum::<Minutes>() + running;
    Minutes((gross - deduction(gross, &rules.tiers)).0.max(0))
}
```

Add to `src/core/mod.rs`: `pub mod balance;` and `pub use balance::{DayStats, Rules, TodayCtx, check_overlap, day_stats, effective_kind, is_working_day, provisional_net, running_balance};`.

- [ ] **Step 4: Run tests**

Run: `nix develop -c cargo test balance`
Expected: 11 passed.

- [ ] **Step 5: Run full check and commit**

Run: `./scripts/check.sh`

```bash
git add src/core
git commit -m "feat(core): balance engine with today rule, overlap, provisional net"
```

---

### Task 8: Config file

**Files:**
- Create: `src/config/mod.rs` (replace stub)

**Interfaces:**
- Consumes: `Rules`, `BreakTier`, `HolidayCalendar`, `Minutes`.
- Produces:
  ```rust
  pub enum ConfigError { Io(std::io::Error), Parse(String), Validation { field: String, reason: String } }
  pub struct BreakTierCfg { pub after_minutes: i32, pub deduct_minutes: i32 }
  pub struct Config { pub start_date: NaiveDate, pub initial_balance_minutes: i32, pub daily_target_minutes: i32,
      pub vacation_days_per_year: u32, pub week_starts_on: String, pub theme: String,
      pub break_tiers: Vec<BreakTierCfg>, pub extra_holidays: Vec<NaiveDate>, pub theme_overrides: BTreeMap<String, String> }
  pub const DEFAULT_TOML: &str
  impl Config { pub fn from_toml(s: &str) -> Result<Config, ConfigError>; pub fn load_or_create(home: &Path) -> Result<Config, ConfigError>;
      pub fn rules(&self) -> Rules; pub fn calendar(&self) -> HolidayCalendar; pub fn validate(&self) -> Result<(), ConfigError> }
  pub fn resolve_home(flag: Option<&Path>) -> PathBuf
  ```

- [ ] **Step 1: Write failing tests**

```rust
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
        assert!(matches!(Config::from_toml(&bad), Err(ConfigError::Validation { .. })));
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
        std::fs::write(home.join("config.toml"), DEFAULT_TOML.replace("= 468", "= 480")).unwrap();
        assert_eq!(Config::load_or_create(&home).unwrap().daily_target_minutes, 480);
    }

    #[test]
    fn extra_holidays_reach_calendar() {
        let toml = DEFAULT_TOML.replace("extra_holidays = []", "extra_holidays = [\"2026-12-24\"]");
        let c = Config::from_toml(&toml).unwrap();
        assert!(c.calendar().is_holiday(NaiveDate::from_ymd_opt(2026, 12, 24).unwrap()));
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
```

- [ ] **Step 2: Run to verify failure**

Run: `nix develop -c cargo test config`
Expected: compile error.

- [ ] **Step 3: Implement `src/config/mod.rs`**

```rust
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use serde::Deserialize;
use thiserror::Error;

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

pub const DEFAULT_TOML: &str = r#"# tk configuration — edit and restart tk
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
"#;

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
    #[serde(default)]
    pub extra_holidays: Vec<NaiveDate>,
    #[serde(default)]
    pub theme_overrides: BTreeMap<String, String>,
}

fn default_vacation() -> u32 { 30 }
fn default_week_start() -> String { "monday".into() }
fn default_theme() -> String { "dark".into() }

fn invalid(field: &str, reason: impl Into<String>) -> ConfigError {
    ConfigError::Validation { field: field.into(), reason: reason.into() }
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
            return Err(invalid("week_starts_on", "must be \"monday\" or \"sunday\""));
        }
        let mut last = -1;
        for t in &self.break_tiers {
            if t.after_minutes < 0 || t.deduct_minutes < 0 {
                return Err(invalid("break_tiers", "values must be non-negative"));
            }
            if t.after_minutes <= last {
                return Err(invalid("break_tiers", "after_minutes must be strictly ascending"));
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
                .map(|t| BreakTier { after: Minutes(t.after_minutes), deduct: Minutes(t.deduct_minutes) })
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
```

- [ ] **Step 4: Run tests**

Run: `nix develop -c cargo test config -- --test-threads=1`
Expected: 5 passed. (`resolve_home` test mutates env; single-threaded is required. Add `#[ignore]`-free but document it in the test name if flaky.)

- [ ] **Step 5: Commit**

```bash
git add src/config
git commit -m "feat(config): TOML config with validation and home resolution"
```

---

### Task 9: SQLite store

**Files:**
- Create: `src/store/mod.rs` (replace stub), `src/store/schema.sql`, `src/store/projects.rs`, `src/store/days.rs`, `src/store/entries.rs`, `src/store/session.rs`

**Interfaces:**
- Consumes: core types, `check_overlap`, `effective_kind`, `HolidayCalendar`.
- Produces:
  ```rust
  pub enum StoreError { Sqlite(rusqlite::Error), Migration(String), NotFound(String), Constraint(String), Core(CoreError) }
  pub struct Store { conn: rusqlite::Connection }
  pub struct Session { pub date: NaiveDate, pub start: NaiveTime, pub project: Option<String> }
  impl Store {
      pub fn open(path: &Path) -> Result<Store, StoreError>;        // creates parent dir, WAL, migrates
      pub fn open_in_memory() -> Result<Store, StoreError>;
      pub fn backup_to(&self, path: &Path) -> Result<(), StoreError>; // VACUUM INTO
      // projects.rs
      pub fn list_projects(&self, include_archived: bool) -> Result<Vec<Project>, StoreError>;
      pub fn project_by_name(&self, name: &str) -> Result<Option<Project>, StoreError>;
      pub fn add_project(&self, name: &str) -> Result<Project, StoreError>;         // Constraint if exists
      pub fn get_or_create_project(&self, name: &str) -> Result<Project, StoreError>;
      pub fn archive_project(&self, name: &str, archived: bool) -> Result<(), StoreError>;
      pub fn rename_project(&self, old: &str, new: &str) -> Result<(), StoreError>;
      pub fn last_used_project(&self) -> Result<Option<String>, StoreError>;
      // days.rs
      pub fn stored_kind(&self, date: NaiveDate) -> Result<Option<DayKind>, StoreError>;
      pub fn set_day_kind(&self, date: NaiveDate, kind: &DayKind) -> Result<(), StoreError>; // Work deletes row; non-work refused if entries exist
      pub fn stored_kinds_in(&self, from: NaiveDate, to: NaiveDate) -> Result<BTreeMap<NaiveDate, DayKind>, StoreError>;
      pub fn days_in(&self, from: NaiveDate, to: NaiveDate, cal: &HolidayCalendar) -> Result<Vec<Day>, StoreError>; // one Day per date inclusive, effective kind
      // entries.rs
      pub fn entries_on(&self, date: NaiveDate) -> Result<Vec<Entry>, StoreError>;       // sorted by start
      pub fn entries_in(&self, from: NaiveDate, to: NaiveDate) -> Result<Vec<Entry>, StoreError>;
      pub fn add_entry(&self, date: NaiveDate, start: NaiveTime, end: NaiveTime, project: &str, comment: &str) -> Result<Entry, StoreError>;
      pub fn update_entry(&self, id: i64, start: NaiveTime, end: NaiveTime, project: &str, comment: &str) -> Result<Entry, StoreError>;
      pub fn delete_entry(&self, id: i64) -> Result<(), StoreError>;
      // session.rs
      pub fn session(&self) -> Result<Option<Session>, StoreError>;
      pub fn clock_in(&self, date: NaiveDate, start: NaiveTime) -> Result<(), StoreError>;   // Constraint if exists
      pub fn clear_session(&self) -> Result<(), StoreError>;
  }
  ```

- [ ] **Step 1: Write `src/store/schema.sql`**

```sql
CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS projects (
    id          INTEGER PRIMARY KEY,
    name        TEXT UNIQUE NOT NULL,
    color_index INTEGER NOT NULL,
    archived    INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS days (
    date  TEXT PRIMARY KEY,
    kind  TEXT NOT NULL,
    label TEXT
);
CREATE TABLE IF NOT EXISTS entries (
    id         INTEGER PRIMARY KEY,
    date       TEXT NOT NULL,
    start_min  INTEGER NOT NULL,
    end_min    INTEGER NOT NULL,
    project_id INTEGER NOT NULL REFERENCES projects(id),
    comment    TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS entries_date ON entries(date);
CREATE TABLE IF NOT EXISTS session (
    id         INTEGER PRIMARY KEY CHECK (id = 1),
    date       TEXT NOT NULL,
    start_min  INTEGER NOT NULL,
    project_id INTEGER
);
INSERT OR IGNORE INTO meta (key, value) VALUES ('schema_version', '1');
```

- [ ] **Step 2: Write failing tests in `src/store/mod.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{DayKind, HolidayCalendar};
    use chrono::{NaiveDate, NaiveTime};

    fn d(y: i32, m: u32, dd: u32) -> NaiveDate { NaiveDate::from_ymd_opt(y, m, dd).unwrap() }
    fn t(h: u32, m: u32) -> NaiveTime { NaiveTime::from_hms_opt(h, m, 0).unwrap() }

    #[test]
    fn opens_and_migrates_file_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("tk.db");
        let s = Store::open(&path).unwrap();
        assert_eq!(s.schema_version().unwrap(), 1);
        drop(s);
        let s = Store::open(&path).unwrap(); // idempotent
        assert_eq!(s.schema_version().unwrap(), 1);
    }

    #[test]
    fn projects_crud_and_colors() {
        let s = Store::open_in_memory().unwrap();
        let a = s.add_project("Alpha").unwrap();
        let b = s.add_project("Beta").unwrap();
        assert_eq!(a.color_index, 0);
        assert_eq!(b.color_index, 1);
        assert!(matches!(s.add_project("Alpha"), Err(StoreError::Constraint(_))));
        assert_eq!(s.get_or_create_project("Alpha").unwrap().id, a.id);
        assert_eq!(s.list_projects(false).unwrap().len(), 2);
        s.archive_project("Beta", true).unwrap();
        assert_eq!(s.list_projects(false).unwrap().len(), 1);
        assert_eq!(s.list_projects(true).unwrap().len(), 2);
        s.rename_project("Alpha", "Alpha2").unwrap();
        assert!(s.project_by_name("Alpha2").unwrap().is_some());
        assert!(matches!(s.rename_project("Nope", "X"), Err(StoreError::NotFound(_))));
    }

    #[test]
    fn entries_crud_overlap_and_ordering() {
        let s = Store::open_in_memory().unwrap();
        let day = d(2026, 9, 14);
        let e2 = s.add_entry(day, t(13, 0), t(17, 0), "Alpha", "pm").unwrap();
        let e1 = s.add_entry(day, t(8, 0), t(12, 0), "Alpha", "am").unwrap();
        let list = s.entries_on(day).unwrap();
        assert_eq!(list.iter().map(|e| e.id).collect::<Vec<_>>(), vec![e1.id, e2.id]);
        assert_eq!(list[0].project, "Alpha");
        assert!(matches!(s.add_entry(day, t(11, 0), t(14, 0), "Beta", ""), Err(StoreError::Core(CoreError::Overlap))));
        // update can move within own slot
        let e1b = s.update_entry(e1.id, t(8, 30), t(12, 0), "Beta", "am2").unwrap();
        assert_eq!(e1b.project, "Beta");
        assert!(matches!(s.update_entry(e1.id, t(8, 0), t(14, 0), "Beta", ""), Err(StoreError::Core(CoreError::Overlap))));
        s.delete_entry(e2.id).unwrap();
        assert_eq!(s.entries_on(day).unwrap().len(), 1);
        assert!(matches!(s.delete_entry(999), Err(StoreError::NotFound(_))));
        assert_eq!(s.last_used_project().unwrap().as_deref(), Some("Beta"));
        assert_eq!(s.entries_in(d(2026, 9, 1), d(2026, 9, 30)).unwrap().len(), 1);
    }

    #[test]
    fn day_kinds_and_entry_constraint() {
        let s = Store::open_in_memory().unwrap();
        let day = d(2026, 9, 14);
        s.set_day_kind(day, &DayKind::Vacation).unwrap();
        assert_eq!(s.stored_kind(day).unwrap(), Some(DayKind::Vacation));
        assert!(matches!(s.add_entry(day, t(8, 0), t(9, 0), "Alpha", ""), Err(StoreError::Constraint(_))));
        s.set_day_kind(day, &DayKind::Work).unwrap();
        assert_eq!(s.stored_kind(day).unwrap(), None);
        s.add_entry(day, t(8, 0), t(9, 0), "Alpha", "").unwrap();
        assert!(matches!(s.set_day_kind(day, &DayKind::Sick), Err(StoreError::Constraint(_))));
        let lbl = DayKind::Absence { label: "training".into() };
        s.set_day_kind(d(2026, 9, 15), &lbl).unwrap();
        assert_eq!(s.stored_kind(d(2026, 9, 15)).unwrap(), Some(lbl));
    }

    #[test]
    fn days_in_assembles_effective_kinds() {
        let s = Store::open_in_memory().unwrap();
        let cal = HolidayCalendar::new(vec![]);
        s.add_entry(d(2026, 12, 24), t(8, 0), t(12, 0), "Alpha", "").unwrap();
        s.set_day_kind(d(2026, 12, 28), &DayKind::Vacation).unwrap();
        let days = s.days_in(d(2026, 12, 24), d(2026, 12, 28), &cal).unwrap();
        assert_eq!(days.len(), 5);
        assert_eq!(days[0].kind, DayKind::Work);
        assert_eq!(days[0].entries.len(), 1);
        assert_eq!(days[1].kind, DayKind::Holiday); // 25 Dec
        assert_eq!(days[4].kind, DayKind::Vacation);
    }

    #[test]
    fn session_lifecycle() {
        let s = Store::open_in_memory().unwrap();
        assert!(s.session().unwrap().is_none());
        s.clock_in(d(2026, 9, 15), t(8, 12)).unwrap();
        let sess = s.session().unwrap().unwrap();
        assert_eq!(sess.start, t(8, 12));
        assert!(matches!(s.clock_in(d(2026, 9, 15), t(9, 0)), Err(StoreError::Constraint(_))));
        s.clear_session().unwrap();
        assert!(s.session().unwrap().is_none());
    }

    #[test]
    fn backup_produces_openable_copy() {
        let dir = tempfile::tempdir().unwrap();
        let s = Store::open(&dir.path().join("tk.db")).unwrap();
        s.add_project("Alpha").unwrap();
        let bak = dir.path().join("bak.db");
        s.backup_to(&bak).unwrap();
        let s2 = Store::open(&bak).unwrap();
        assert_eq!(s2.list_projects(true).unwrap().len(), 1);
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `nix develop -c cargo test store`
Expected: compile error.

- [ ] **Step 4: Implement `src/store/mod.rs`**

```rust
mod days;
mod entries;
mod projects;
mod session;

use std::path::Path;

use rusqlite::Connection;
use thiserror::Error;

use crate::core::CoreError;

pub use session::Session;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("migration error: {0}")]
    Migration(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("{0}")]
    Constraint(String),
    #[error("{0}")]
    Core(#[from] CoreError),
}

pub type StoreResult<T> = Result<T, StoreError>;

pub struct Store {
    conn: Connection,
}

const SCHEMA: &str = include_str!("schema.sql");

impl Store {
    pub fn open(path: &Path) -> StoreResult<Store> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| StoreError::Migration(e.to_string()))?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> StoreResult<Store> {
        let conn = Connection::open_in_memory()?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Self::init(conn)
    }

    fn init(conn: Connection) -> StoreResult<Store> {
        conn.execute_batch(SCHEMA)?;
        let store = Store { conn };
        match store.schema_version()? {
            1 => Ok(store),
            v => Err(StoreError::Migration(format!("unsupported schema version {v}"))),
        }
    }

    pub fn schema_version(&self) -> StoreResult<i64> {
        let v: String = self
            .conn
            .query_row("SELECT value FROM meta WHERE key = 'schema_version'", [], |r| r.get(0))?;
        v.parse().map_err(|_| StoreError::Migration(format!("bad schema_version {v}")))
    }

    pub fn backup_to(&self, path: &Path) -> StoreResult<()> {
        let p = path.to_string_lossy().to_string();
        self.conn.execute("VACUUM INTO ?1", [p])?;
        Ok(())
    }

    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }
}

pub(crate) fn date_str(d: chrono::NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

pub(crate) fn parse_date(s: &str) -> rusqlite::Result<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

pub(crate) fn time_from_min(m: i64) -> chrono::NaiveTime {
    chrono::NaiveTime::from_hms_opt((m / 60) as u32, (m % 60) as u32, 0).expect("stored minutes valid")
}
```

- [ ] **Step 5: Implement `src/store/projects.rs`**

```rust
use rusqlite::{OptionalExtension, params};

use super::{Store, StoreError, StoreResult};
use crate::core::Project;

const PALETTE_SIZE: u8 = 10;

fn row_to_project(r: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: r.get(0)?,
        name: r.get(1)?,
        color_index: r.get::<_, i64>(2)? as u8,
        archived: r.get::<_, i64>(3)? != 0,
    })
}

impl Store {
    pub fn list_projects(&self, include_archived: bool) -> StoreResult<Vec<Project>> {
        let sql = if include_archived {
            "SELECT id, name, color_index, archived FROM projects ORDER BY name"
        } else {
            "SELECT id, name, color_index, archived FROM projects WHERE archived = 0 ORDER BY name"
        };
        let mut st = self.conn().prepare(sql)?;
        let rows = st.query_map([], row_to_project)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn project_by_name(&self, name: &str) -> StoreResult<Option<Project>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT id, name, color_index, archived FROM projects WHERE name = ?1",
                [name],
                row_to_project,
            )
            .optional()?)
    }

    pub fn add_project(&self, name: &str) -> StoreResult<Project> {
        let name = name.trim();
        if name.is_empty() {
            return Err(StoreError::Constraint("project name must not be empty".into()));
        }
        if self.project_by_name(name)?.is_some() {
            return Err(StoreError::Constraint(format!("project '{name}' already exists")));
        }
        let count: i64 = self.conn().query_row("SELECT COUNT(*) FROM projects", [], |r| r.get(0))?;
        let color = (count as u8) % PALETTE_SIZE;
        self.conn().execute(
            "INSERT INTO projects (name, color_index, archived) VALUES (?1, ?2, 0)",
            params![name, color as i64],
        )?;
        Ok(self.project_by_name(name)?.expect("just inserted"))
    }

    pub fn get_or_create_project(&self, name: &str) -> StoreResult<Project> {
        match self.project_by_name(name.trim())? {
            Some(p) => Ok(p),
            None => self.add_project(name),
        }
    }

    pub fn archive_project(&self, name: &str, archived: bool) -> StoreResult<()> {
        let n = self.conn().execute(
            "UPDATE projects SET archived = ?2 WHERE name = ?1",
            params![name, archived as i64],
        )?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("project '{name}'")));
        }
        Ok(())
    }

    pub fn rename_project(&self, old: &str, new: &str) -> StoreResult<()> {
        if self.project_by_name(new)?.is_some() {
            return Err(StoreError::Constraint(format!("project '{new}' already exists")));
        }
        let n = self
            .conn()
            .execute("UPDATE projects SET name = ?2 WHERE name = ?1", params![old, new.trim()])?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("project '{old}'")));
        }
        Ok(())
    }

    pub fn last_used_project(&self) -> StoreResult<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT p.name FROM entries e JOIN projects p ON p.id = e.project_id
                 ORDER BY e.date DESC, e.start_min DESC, e.id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?)
    }
}
```

- [ ] **Step 6: Implement `src/store/days.rs`**

```rust
use std::collections::BTreeMap;

use chrono::{Days, NaiveDate};
use rusqlite::{OptionalExtension, params};

use super::{Store, StoreError, StoreResult, date_str, parse_date};
use crate::core::{Day, DayKind, HolidayCalendar, effective_kind};

fn kind_from_row(kind: String, label: Option<String>) -> DayKind {
    DayKind::parse(&kind, label.as_deref()).unwrap_or(DayKind::Work)
}

impl Store {
    pub fn stored_kind(&self, date: NaiveDate) -> StoreResult<Option<DayKind>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT kind, label FROM days WHERE date = ?1",
                [date_str(date)],
                |r| Ok(kind_from_row(r.get(0)?, r.get(1)?)),
            )
            .optional()?)
    }

    pub fn set_day_kind(&self, date: NaiveDate, kind: &DayKind) -> StoreResult<()> {
        let ds = date_str(date);
        if *kind == DayKind::Work {
            self.conn().execute("DELETE FROM days WHERE date = ?1", [ds])?;
            return Ok(());
        }
        let n: i64 = self
            .conn()
            .query_row("SELECT COUNT(*) FROM entries WHERE date = ?1", [&ds], |r| r.get(0))?;
        if n > 0 {
            return Err(StoreError::Constraint(format!(
                "{ds} has {n} time entr{}; remove them before changing the day type",
                if n == 1 { "y" } else { "ies" }
            )));
        }
        self.conn().execute(
            "INSERT INTO days (date, kind, label) VALUES (?1, ?2, ?3)
             ON CONFLICT(date) DO UPDATE SET kind = excluded.kind, label = excluded.label",
            params![ds, kind.as_str(), kind.label()],
        )?;
        Ok(())
    }

    pub fn stored_kinds_in(&self, from: NaiveDate, to: NaiveDate) -> StoreResult<BTreeMap<NaiveDate, DayKind>> {
        let mut st = self
            .conn()
            .prepare("SELECT date, kind, label FROM days WHERE date BETWEEN ?1 AND ?2")?;
        let rows = st.query_map([date_str(from), date_str(to)], |r| {
            Ok((parse_date(&r.get::<_, String>(0)?)?, kind_from_row(r.get(1)?, r.get(2)?)))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn days_in(&self, from: NaiveDate, to: NaiveDate, cal: &HolidayCalendar) -> StoreResult<Vec<Day>> {
        let kinds = self.stored_kinds_in(from, to)?;
        let mut by_date: BTreeMap<NaiveDate, Vec<_>> = BTreeMap::new();
        for e in self.entries_in(from, to)? {
            by_date.entry(e.date).or_default().push(e);
        }
        let mut out = Vec::new();
        let mut d = from;
        while d <= to {
            out.push(Day {
                date: d,
                kind: effective_kind(kinds.get(&d), d, cal),
                entries: by_date.remove(&d).unwrap_or_default(),
            });
            d = d.checked_add_days(Days::new(1)).expect("date range");
        }
        Ok(out)
    }
}
```

- [ ] **Step 7: Implement `src/store/entries.rs`**

```rust
use chrono::{NaiveDate, NaiveTime};
use rusqlite::params;

use super::{Store, StoreError, StoreResult, date_str, parse_date, time_from_min};
use crate::core::{DayKind, Entry, check_overlap, minutes_of};

const SELECT: &str = "SELECT e.id, e.date, e.start_min, e.end_min, p.name, e.comment
                      FROM entries e JOIN projects p ON p.id = e.project_id";

fn row_to_entry(r: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: r.get(0)?,
        date: parse_date(&r.get::<_, String>(1)?)?,
        start: time_from_min(r.get(2)?),
        end: time_from_min(r.get(3)?),
        project: r.get(4)?,
        comment: r.get(5)?,
    })
}

impl Store {
    pub fn entries_on(&self, date: NaiveDate) -> StoreResult<Vec<Entry>> {
        let mut st = self.conn().prepare(&format!("{SELECT} WHERE e.date = ?1 ORDER BY e.start_min, e.id"))?;
        let rows = st.query_map([date_str(date)], row_to_entry)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn entries_in(&self, from: NaiveDate, to: NaiveDate) -> StoreResult<Vec<Entry>> {
        let mut st = self.conn().prepare(&format!(
            "{SELECT} WHERE e.date BETWEEN ?1 AND ?2 ORDER BY e.date, e.start_min, e.id"
        ))?;
        let rows = st.query_map([date_str(from), date_str(to)], row_to_entry)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn entry(&self, id: i64) -> StoreResult<Entry> {
        self.conn()
            .query_row(&format!("{SELECT} WHERE e.id = ?1"), [id], row_to_entry)
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => StoreError::NotFound(format!("entry {id}")),
                other => StoreError::Sqlite(other),
            })
    }

    fn ensure_work_day(&self, date: NaiveDate) -> StoreResult<()> {
        if let Some(k) = self.stored_kind(date)?
            && k != DayKind::Work
        {
            return Err(StoreError::Constraint(format!(
                "{} is a {} day; set it to work first",
                date_str(date),
                k.display_name().to_lowercase()
            )));
        }
        Ok(())
    }

    pub fn add_entry(
        &self,
        date: NaiveDate,
        start: NaiveTime,
        end: NaiveTime,
        project: &str,
        comment: &str,
    ) -> StoreResult<Entry> {
        self.ensure_work_day(date)?;
        check_overlap(&self.entries_on(date)?, start, end, None)?;
        let p = self.get_or_create_project(project)?;
        self.conn().execute(
            "INSERT INTO entries (date, start_min, end_min, project_id, comment) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![date_str(date), minutes_of(start), minutes_of(end), p.id, comment.trim()],
        )?;
        self.entry(self.conn().last_insert_rowid())
    }

    pub fn update_entry(
        &self,
        id: i64,
        start: NaiveTime,
        end: NaiveTime,
        project: &str,
        comment: &str,
    ) -> StoreResult<Entry> {
        let existing = self.entry(id)?;
        check_overlap(&self.entries_on(existing.date)?, start, end, Some(id))?;
        let p = self.get_or_create_project(project)?;
        self.conn().execute(
            "UPDATE entries SET start_min = ?2, end_min = ?3, project_id = ?4, comment = ?5 WHERE id = ?1",
            params![id, minutes_of(start), minutes_of(end), p.id, comment.trim()],
        )?;
        self.entry(id)
    }

    pub fn delete_entry(&self, id: i64) -> StoreResult<()> {
        let n = self.conn().execute("DELETE FROM entries WHERE id = ?1", [id])?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("entry {id}")));
        }
        Ok(())
    }
}
```

- [ ] **Step 8: Implement `src/store/session.rs`**

```rust
use chrono::{NaiveDate, NaiveTime};
use rusqlite::{OptionalExtension, params};

use super::{Store, StoreError, StoreResult, date_str, parse_date, time_from_min};
use crate::core::minutes_of;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub date: NaiveDate,
    pub start: NaiveTime,
    pub project: Option<String>,
}

impl Store {
    pub fn session(&self) -> StoreResult<Option<Session>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT s.date, s.start_min, p.name FROM session s
                 LEFT JOIN projects p ON p.id = s.project_id WHERE s.id = 1",
                [],
                |r| {
                    Ok(Session {
                        date: parse_date(&r.get::<_, String>(0)?)?,
                        start: time_from_min(r.get(1)?),
                        project: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn clock_in(&self, date: NaiveDate, start: NaiveTime) -> StoreResult<()> {
        if let Some(s) = self.session()? {
            return Err(StoreError::Constraint(format!(
                "already clocked in since {} {}",
                date_str(s.date),
                s.start.format("%H:%M")
            )));
        }
        self.conn().execute(
            "INSERT INTO session (id, date, start_min, project_id) VALUES (1, ?1, ?2, NULL)",
            params![date_str(date), minutes_of(start)],
        )?;
        Ok(())
    }

    pub fn clear_session(&self) -> StoreResult<()> {
        self.conn().execute("DELETE FROM session WHERE id = 1", [])?;
        Ok(())
    }
}
```

- [ ] **Step 9: Run tests**

Run: `nix develop -c cargo test store`
Expected: 7 passed.

- [ ] **Step 10: Check and commit**

Run: `./scripts/check.sh`

```bash
git add src/store
git commit -m "feat(store): SQLite store with projects, days, entries, session"
```

---

### Task 10: CLI commands

**Files:**
- Create: `src/cli/mod.rs` (replace stub), `src/cli/commands.rs`, `tests/cli.rs`
- Modify: `src/main.rs`

**Interfaces:**
- Consumes: `Config`, `Store`, core parsing and balance functions.
- Produces:
  ```rust
  // src/cli/mod.rs
  #[derive(clap::Parser)] pub struct Cli { #[arg(long, global = true)] pub home: Option<PathBuf>, #[command(subcommand)] pub cmd: Option<Command> }
  #[derive(clap::Subcommand)] pub enum Command { In { #[arg(long)] force: bool }, Out { #[arg(short, long)] project: Option<String>, #[arg(short = 'm', long)] comment: Option<String> },
      Status, Add { date: String, range: String, #[arg(short, long)] project: String, #[arg(short = 'm', long, default_value = "")] comment: String },
      Day { date: String, kind: String, #[arg(long)] to: Option<String>, #[arg(long)] label: Option<String> },
      Projects { #[command(subcommand)] action: Option<ProjectAction> }, Backup, Export { #[arg(long, default_value = "csv")] format: String, #[arg(long)] from: Option<String>, #[arg(long)] to: Option<String>, #[arg(short, long)] output: Option<PathBuf> } }
  #[derive(clap::Subcommand)] pub enum ProjectAction { List, Add { name: String }, Archive { name: String }, Unarchive { name: String }, Rename { old: String, new: String } }
  pub struct Ctx { pub home: PathBuf, pub config: Config, pub store: Store }
  pub fn open_ctx(home: Option<&Path>) -> anyhow::Result<Ctx>
  // src/cli/commands.rs
  pub fn run(cmd: Command, ctx: &Ctx, out: &mut dyn std::io::Write) -> anyhow::Result<()>
  pub fn status_line(ctx: &Ctx, now: chrono::NaiveDateTime) -> anyhow::Result<String>
  pub fn balance_as_of(ctx: &Ctx, today: NaiveDate, clocked_in: bool) -> anyhow::Result<Minutes>  // running balance start_date..=today
  ```

- [ ] **Step 1: Write failing integration tests `tests/cli.rs`**

```rust
use assert_cmd::Command;
use predicates::prelude::*;

fn tk(home: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("tk").unwrap();
    c.env("TK_HOME", home);
    c
}

#[test]
fn first_run_creates_home_and_config() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("h");
    tk(&home).arg("status").assert().success().stdout(predicate::str::contains("not clocked in"));
    assert!(home.join("config.toml").exists());
    assert!(home.join("tk.db").exists());
}

#[test]
fn add_day_and_status_and_export() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    tk(home).args(["add", "2026-09-14", "0900-1530", "-p", "Alpha", "-m", "note"]).assert().success()
        .stdout(predicate::str::contains("+05:42"));
    tk(home).args(["add", "2026-09-14", "1000-1100", "-p", "Alpha"]).assert().failure()
        .stderr(predicate::str::contains("overlap"));
    tk(home).args(["day", "2026-09-15", "vacation", "--to", "2026-09-16"]).assert().success()
        .stdout(predicate::str::contains("2 day"));
    tk(home).args(["day", "2026-09-14", "sick"]).assert().failure();
    tk(home).args(["projects"]).assert().success().stdout(predicate::str::contains("Alpha"));
    tk(home).args(["export", "--format", "csv"]).assert().success()
        .stdout(predicate::str::contains("2026-09-14,09:00,15:30,Alpha,note"));
    tk(home).args(["export", "--format", "json"]).assert().success()
        .stdout(predicate::str::contains("\"project\":\"Alpha\""));
}

#[test]
fn clock_in_and_out() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    tk(home).arg("in").assert().success().stdout(predicate::str::contains("Clocked in"));
    tk(home).arg("in").assert().failure().stderr(predicate::str::contains("already clocked in"));
    tk(home).arg("status").assert().success().stdout(predicate::str::contains("⏱"));
    tk(home).args(["out", "-p", "Beta", "-m", "x"]).assert().success().stdout(predicate::str::contains("Beta"));
    tk(home).arg("out").assert().failure().stderr(predicate::str::contains("not clocked in"));
    tk(home).arg("in").assert().success();
    tk(home).args(["out"]).assert().success().stdout(predicate::str::contains("Beta")); // last-used project
}

#[test]
fn backup_writes_file() {
    let dir = tempfile::tempdir().unwrap();
    tk(dir.path()).arg("backup").assert().success().stdout(predicate::str::contains("backups/tk-"));
    assert_eq!(std::fs::read_dir(dir.path().join("backups")).unwrap().count(), 1);
}

#[test]
fn bad_config_exits_2() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("config.toml"), "start_date = \"2026-01-01\"\ndaily_target_minutes = 0\n").unwrap();
    tk(dir.path()).arg("status").assert().code(2).stderr(predicate::str::contains("daily_target_minutes"));
}
```

- [ ] **Step 2: Run to verify failure**

Run: `nix develop -c cargo test --test cli`
Expected: failures (binary prints `tk 0.1.0` and ignores args).

- [ ] **Step 3: Implement `src/cli/mod.rs`**

```rust
pub mod commands;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use crate::config::{Config, ConfigError, resolve_home};
use crate::store::Store;

#[derive(Parser, Debug)]
#[command(name = "tk", version, about = "Flexitime tracker. Run without a command to open the TUI.")]
pub struct Cli {
    /// Data directory (default: $TK_HOME or ~/.local/share/tk)
    #[arg(long, global = true, value_name = "DIR")]
    pub home: Option<PathBuf>,
    #[command(subcommand)]
    pub cmd: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Clock in now
    In {
        /// Replace an existing clock-in
        #[arg(long)]
        force: bool,
    },
    /// Clock out and record the entry
    Out {
        #[arg(short, long)]
        project: Option<String>,
        #[arg(short = 'm', long)]
        comment: Option<String>,
    },
    /// One-line status for prompts and status bars
    Status,
    /// Add an entry: tk add 2026-09-14 0900-1530 -p Alpha -m "note"
    Add {
        date: String,
        range: String,
        #[arg(short, long)]
        project: String,
        #[arg(short = 'm', long, default_value = "")]
        comment: String,
    },
    /// Set a day kind: work | vacation | flex | holiday | sick | absence
    Day {
        date: String,
        kind: String,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        label: Option<String>,
    },
    /// Manage projects
    Projects {
        #[command(subcommand)]
        action: Option<ProjectAction>,
    },
    /// Copy the database into backups/
    Backup,
    /// Export entries as CSV or JSON
    Export {
        #[arg(long, default_value = "csv")]
        format: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ProjectAction {
    List,
    Add { name: String },
    Archive { name: String },
    Unarchive { name: String },
    Rename { old: String, new: String },
}

pub struct Ctx {
    pub home: PathBuf,
    pub config: Config,
    pub store: Store,
}

/// Exit code 2 for config problems, 1 for everything else.
pub struct ExitCode(pub i32);

pub fn open_ctx(home: Option<&Path>) -> Result<Ctx, (ExitCode, anyhow::Error)> {
    let home = resolve_home(home);
    let config = Config::load_or_create(&home).map_err(|e: ConfigError| (ExitCode(2), e.into()))?;
    let store = Store::open(&home.join("tk.db")).map_err(|e| (ExitCode(1), e.into()))?;
    Ok(Ctx { home, config, store })
}
```

- [ ] **Step 4: Implement `src/cli/commands.rs`**

```rust
use std::io::Write;

use anyhow::{Context as _, anyhow, bail};
use chrono::{Days, Local, NaiveDate, NaiveDateTime};

use super::{Command, Ctx, ProjectAction};
use crate::core::{DayKind, Minutes, day_stats, parse_date, parse_time_range, provisional_net, running_balance, TodayCtx};

pub fn now_local() -> NaiveDateTime {
    Local::now().naive_local()
}

pub fn balance_as_of(ctx: &Ctx, today: NaiveDate, clocked_in: bool) -> anyhow::Result<Minutes> {
    let rules = ctx.config.rules();
    let cal = ctx.config.calendar();
    let days = ctx.store.days_in(rules.start_date, today, &cal)?;
    let tc = TodayCtx { today, clocked_in };
    let stats: Vec<_> = days.iter().map(|d| day_stats(d, &rules, &cal, &tc)).collect();
    Ok(running_balance(&stats, &rules))
}

pub fn status_line(ctx: &Ctx, now: NaiveDateTime) -> anyhow::Result<String> {
    let today = now.date();
    let rules = ctx.config.rules();
    let session = ctx.store.session()?;
    let balance = balance_as_of(ctx, today, session.is_some())?;
    let entries = ctx.store.entries_on(today)?;
    Ok(match session {
        Some(s) => {
            let running = Minutes(((now.time() - s.start).num_minutes()).max(0) as i32);
            let net = provisional_net(&entries, s.start, now.time(), &rules);
            format!(
                "⏱ {} (in {}) · today {} · balance {}",
                running.hhmm(),
                s.start.format("%H:%M"),
                net,
                balance
            )
        }
        None => {
            let net: Minutes = {
                let cal = ctx.config.calendar();
                let d = ctx.store.days_in(today, today, &cal)?.remove(0);
                day_stats(&d, &rules, &cal, &TodayCtx { today, clocked_in: false }).net
            };
            format!("not clocked in · today {net} · balance {balance}")
        }
    })
}

pub fn run(cmd: Command, ctx: &Ctx, out: &mut dyn Write) -> anyhow::Result<()> {
    let now = now_local();
    let today = now.date();
    match cmd {
        Command::In { force } => {
            if force {
                ctx.store.clear_session()?;
            }
            let t = now.time().with_second(0).unwrap_or(now.time());
            ctx.store.clock_in(today, t)?;
            writeln!(out, "Clocked in at {}", t.format("%H:%M"))?;
        }
        Command::Out { project, comment } => {
            let s = ctx.store.session()?.ok_or_else(|| anyhow!("not clocked in"))?;
            let project = match project.or(ctx.store.last_used_project()?) {
                Some(p) => p,
                None => bail!("no project given and none used before; pass --project NAME"),
            };
            let end = now.time().with_second(0).unwrap_or(now.time());
            let e = ctx.store.add_entry(s.date, s.start, end, &project, comment.as_deref().unwrap_or(""))?;
            ctx.store.clear_session()?;
            let stats = day_stats(
                &ctx.store.days_in(s.date, s.date, &ctx.config.calendar())?.remove(0),
                &ctx.config.rules(), &ctx.config.calendar(), &TodayCtx { today, clocked_in: false },
            );
            writeln!(
                out,
                "Recorded {}–{} {} ({}) · day net {} · balance {}",
                e.start.format("%H:%M"), e.end.format("%H:%M"), e.project, e.duration(),
                stats.net, balance_as_of(ctx, today, false)?
            )?;
        }
        Command::Status => writeln!(out, "{}", status_line(ctx, now)?)?,
        Command::Add { date, range, project, comment } => {
            let date = parse_date(&date, today)?;
            let (s, e) = parse_time_range(&range)?;
            let entry = ctx.store.add_entry(date, s, e, &project, &comment)?;
            let day = ctx.store.days_in(date, date, &ctx.config.calendar())?.remove(0);
            let st = day_stats(&day, &ctx.config.rules(), &ctx.config.calendar(), &TodayCtx { today, clocked_in: false });
            writeln!(
                out,
                "Added {} {}–{} {} · gross {} · day net {}",
                date, entry.start.format("%H:%M"), entry.end.format("%H:%M"), entry.project, entry.duration(), st.net
            )?;
        }
        Command::Day { date, kind, to, label } => {
            let from = parse_date(&date, today)?;
            let to = match to { Some(t) => parse_date(&t, today)?, None => from };
            if to < from { bail!("--to must not be before DATE"); }
            let kind = DayKind::parse(&kind, label.as_deref())
                .ok_or_else(|| anyhow!("unknown kind '{kind}'; use work|vacation|flex|holiday|sick|absence"))?;
            let mut d = from;
            let mut n = 0;
            while d <= to {
                ctx.store.set_day_kind(d, &kind).with_context(|| format!("{d}"))?;
                n += 1;
                d = d.checked_add_days(Days::new(1)).unwrap();
            }
            writeln!(out, "Set {} day{} to {}", n, if n == 1 { "" } else { "s" }, kind.display_name().to_lowercase())?;
        }
        Command::Projects { action } => match action.unwrap_or(ProjectAction::List) {
            ProjectAction::List => {
                for p in ctx.store.list_projects(true)? {
                    writeln!(out, "{}{}", p.name, if p.archived { "  (archived)" } else { "" })?;
                }
            }
            ProjectAction::Add { name } => { ctx.store.add_project(&name)?; writeln!(out, "Added {name}")?; }
            ProjectAction::Archive { name } => { ctx.store.archive_project(&name, true)?; writeln!(out, "Archived {name}")?; }
            ProjectAction::Unarchive { name } => { ctx.store.archive_project(&name, false)?; writeln!(out, "Unarchived {name}")?; }
            ProjectAction::Rename { old, new } => { ctx.store.rename_project(&old, &new)?; writeln!(out, "Renamed {old} → {new}")?; }
        },
        Command::Backup => {
            let dir = ctx.home.join("backups");
            std::fs::create_dir_all(&dir)?;
            let path = dir.join(format!("tk-{}.db", now.format("%Y%m%d-%H%M%S")));
            ctx.store.backup_to(&path)?;
            writeln!(out, "Backup written to {}", path.display())?;
        }
        Command::Export { format, from, to, output } => {
            let rules = ctx.config.rules();
            let from = match from { Some(f) => parse_date(&f, today)?, None => rules.start_date };
            let to = match to { Some(t) => parse_date(&t, today)?, None => today };
            let entries = ctx.store.entries_in(from, to)?;
            let mut text = String::new();
            match format.as_str() {
                "csv" => {
                    text.push_str("date,start,end,project,comment,gross\n");
                    for e in &entries {
                        text.push_str(&format!(
                            "{},{},{},{},{},{}\n",
                            e.date, e.start.format("%H:%M"), e.end.format("%H:%M"),
                            csv_quote(&e.project), csv_quote(&e.comment), e.duration()
                        ));
                    }
                }
                "json" => {
                    text.push('[');
                    for (i, e) in entries.iter().enumerate() {
                        if i > 0 { text.push(','); }
                        text.push_str(&format!(
                            "{{\"date\":\"{}\",\"start\":\"{}\",\"end\":\"{}\",\"project\":{},\"comment\":{},\"gross_minutes\":{}}}",
                            e.date, e.start.format("%H:%M"), e.end.format("%H:%M"),
                            json_quote(&e.project), json_quote(&e.comment), e.duration().0
                        ));
                    }
                    text.push_str("]\n");
                }
                other => bail!("unknown format '{other}'; use csv or json"),
            }
            match output {
                Some(p) => { std::fs::write(&p, text)?; writeln!(out, "Wrote {}", p.display())?; }
                None => out.write_all(text.as_bytes())?,
            }
        }
    }
    Ok(())
}

fn csv_quote(s: &str) -> String {
    if s.contains([',', '"', '\n']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

fn json_quote(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}
```

Add `use chrono::Timelike;` at the top for `with_second`.

- [ ] **Step 5: Implement `src/main.rs`**

```rust
use clap::Parser;
use tk::cli::{Cli, ExitCode, commands, open_ctx};

fn main() {
    let cli = Cli::parse();
    let ctx = match open_ctx(cli.home.as_deref()) {
        Ok(c) => c,
        Err((ExitCode(code), err)) => {
            eprintln!("error: {err:#}");
            std::process::exit(code);
        }
    };
    let result = match cli.cmd {
        None => tk::tui::run(ctx),
        Some(cmd) => {
            let stdout = std::io::stdout();
            let mut lock = stdout.lock();
            commands::run(cmd, &ctx, &mut lock)
        }
    };
    if let Err(err) = result {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}
```

And in `src/tui/mod.rs` for now:
```rust
//! TUI (implemented from Task 12 on)
use crate::cli::Ctx;

pub fn run(_ctx: Ctx) -> anyhow::Result<()> {
    anyhow::bail!("TUI not implemented yet; use `tk --help`")
}
```

- [ ] **Step 6: Run tests**

Run: `nix develop -c cargo test --test cli`
Expected: 5 passed. Check `error:` lines contain the lowercase words the tests look for (`overlap`, `already clocked in`, `not clocked in`); CoreError::Overlap's message is "entry overlaps an existing entry", which contains "overlap".

- [ ] **Step 7: Check and commit**

Run: `./scripts/check.sh`

```bash
git add src/cli src/main.rs src/tui tests/cli.rs
git commit -m "feat(cli): in/out/status/add/day/projects/backup/export"
```

---

## TUI architecture notes (read before Tasks 11–19)

tui-realm 4.1 facts verified from the crate source (`~/.cargo/registry/src/*/tuirealm-4.1.0/`):

- `Application<Id, Msg, UserEvent>` where `UserEvent: Eq + PartialEq + Clone + Send + 'static`. `Msg: PartialEq`. `Id: Eq + Hash + Clone`.
- Keyboard events go to the **active** component only. Components subscribed via `Sub::new(EventClause::Tick, SubClause::Always)` or `EventClause::User(...)` also receive those events. `EventClause::User(ev)` matches by `PartialEq`, so `UserEvent` compares **discriminants only** (see `msg.rs`).
- Component = `impl Component` (`view`, `query`, `attr`, `state`, `perform`) + `impl AppComponent<Msg, UserEvent>` (`fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg>`). `ev.as_keyboard()` gives `Option<&KeyEvent>`; `KeyEvent { code: Key, modifiers: KeyModifiers }`; `Key::Char(c)`, `Key::Up/Down/Left/Right/Enter/Esc/Tab/BackTab/Backspace/Delete`.
- Async: `EventListenerCfg::default().with_handle(tokio::runtime::Handle).async_crossterm_input_listener(Duration::ZERO, 3).add_async_port(Box::new(port), Duration::ZERO, 1).tick_interval(Duration::from_secs(1))`. A `PollAsync` port's `poll()` **awaits until an event is available** and returns `Ok(Some(Event::User(..)))`; returning `Ok(None)` stops the port forever.
- Terminal: `CrosstermTerminalAdapter::new()`, `.enable_raw_mode()`, `.enter_alternate_screen()`, `.draw(|f| ..)`, `.restore()`.
- Main loop: `app.tick(PollStrategy::Once(Duration::from_millis(10)))` returns `Vec<Msg>`; call `model.update(msg)` for each; redraw when `model.redraw`.
- stdlib: `tui_realm_stdlib::Input` (builder: `.title(..)`, `.value(..)`, `.placeholder(..)`, `.borders(..)`, `.foreground(..)`, `.invalid_style(..)`), `Phantom` (invisible component for subscriptions), `Radio`, `Table`. In this plan only `Input` and `Phantom` are used; everything else is drawn by pure functions with plain ratatui widgets.

**Design used by every TUI task:** the `Model` owns all state and draws everything itself with pure `draw_*` functions (so snapshot tests never need tui-realm). tui-realm components are thin: they hold no state except the form inputs, and their only job is to turn key events into `Msg`s and to own focus. Exactly one "screen root" component is active at any time; overlays (form, confirm, help) become active while open and `Esc` returns focus.

Snapshot testing helper (used from Task 11 on), in `src/tui/view/mod.rs` under `#[cfg(test)]`:

```rust
#[cfg(test)]
pub(crate) mod testing {
    use tuirealm::ratatui::backend::TestBackend;
    use tuirealm::ratatui::{Frame, Terminal};

    /// Render with `f` into a WxH buffer and return the text rows (trailing spaces trimmed).
    pub fn render(w: u16, h: u16, f: impl FnOnce(&mut Frame)) -> Vec<String> {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(f).unwrap();
        let buf = term.backend().buffer().clone();
        (0..h)
            .map(|y| {
                let s: String = (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect();
                s.trim_end().to_string()
            })
            .collect()
    }

    pub fn contains(rows: &[String], needle: &str) -> bool {
        rows.iter().any(|r| r.contains(needle))
    }
}
```

---

### Task 11: Theme and shared drawing helpers

**Files:**
- Create: `src/tui/theme.rs`, `src/tui/view/mod.rs`
- Modify: `src/tui/mod.rs` (add `pub mod theme; pub mod view;`)

**Interfaces:**
- Consumes: `Config` (`theme`, `theme_overrides`), `Minutes`, `DayKind`.
- Produces:
  ```rust
  pub struct Theme { pub positive: Color, pub negative: Color, pub warning: Color, pub accent: Color, pub muted: Color, pub text: Color,
      pub bg_selected: Color, pub chip_vacation: Color, pub chip_flex: Color, pub chip_holiday: Color, pub chip_sick: Color, pub chip_absence: Color,
      pub projects: [Color; 10], pub border: BorderType }
  impl Theme { pub fn dark() -> Theme; pub fn light() -> Theme; pub fn from_config(cfg: &Config) -> Theme;
      pub fn project_color(&self, idx: u8) -> Color; pub fn kind_color(&self, k: &DayKind) -> Color;
      pub fn minutes_style(&self, m: Minutes) -> Style; pub fn downgrade_for_terminal(self) -> Theme }
  pub fn parse_color(s: &str) -> Option<Color>   // "#rrggbb" or ratatui named color
  pub fn supports_truecolor() -> bool             // COLORTERM = truecolor | 24bit
  // view/mod.rs
  pub fn minutes_span(m: Minutes, theme: &Theme) -> Span<'static>
  pub fn chip(text: &str, color: Color) -> Span<'static>          // " TEXT " bold on colored bg
  pub fn bar(frac: f64, width: u16, color: Color, theme: &Theme) -> Line<'static>  // █ and ░
  pub fn block(theme: &Theme, title: Option<&str>) -> Block<'static> // rounded, muted border
  ```

- [ ] **Step 1: Write failing tests (`src/tui/theme.rs`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, DEFAULT_TOML};
    use tuirealm::ratatui::style::Color;

    #[test]
    fn parses_hex_and_named() {
        assert_eq!(parse_color("#a6e3a1"), Some(Color::Rgb(0xa6, 0xe3, 0xa1)));
        assert_eq!(parse_color("red"), Some(Color::Red));
        assert_eq!(parse_color("nope"), None);
    }

    #[test]
    fn overrides_apply() {
        let toml = DEFAULT_TOML.replace("# positive = \"#a6e3a1\"", "positive = \"#123456\"");
        let cfg = Config::from_toml(&toml).unwrap();
        let t = Theme::from_config(&cfg);
        assert_eq!(t.positive, Color::Rgb(0x12, 0x34, 0x56));
        assert_ne!(t.negative, t.positive);
    }

    #[test]
    fn project_colors_cycle() {
        let t = Theme::dark();
        assert_eq!(t.project_color(0), t.projects[0]);
        assert_eq!(t.project_color(13), t.projects[3]);
    }

    #[test]
    fn minutes_style_sign() {
        let t = Theme::dark();
        assert_eq!(t.minutes_style(Minutes(5)).fg, Some(t.positive));
        assert_eq!(t.minutes_style(Minutes(-5)).fg, Some(t.negative));
        assert_eq!(t.minutes_style(Minutes(0)).fg, Some(t.muted));
    }

    #[test]
    fn downgrade_maps_rgb_to_indexed() {
        let t = Theme::dark().downgrade_for_terminal();
        assert!(matches!(t.positive, Color::Indexed(_)));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `nix develop -c cargo test tui::theme`
Expected: compile error.

- [ ] **Step 3: Implement `src/tui/theme.rs`**

```rust
use std::str::FromStr;

use tuirealm::ratatui::style::{Color, Modifier, Style};
use tuirealm::ratatui::widgets::BorderType;

use crate::config::Config;
use crate::core::{DayKind, Minutes};

#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub positive: Color,
    pub negative: Color,
    pub warning: Color,
    pub accent: Color,
    pub muted: Color,
    pub text: Color,
    pub bg_selected: Color,
    pub chip_vacation: Color,
    pub chip_flex: Color,
    pub chip_holiday: Color,
    pub chip_sick: Color,
    pub chip_absence: Color,
    pub projects: [Color; 10],
    pub border: BorderType,
}

fn rgb(hex: u32) -> Color {
    Color::Rgb(((hex >> 16) & 0xff) as u8, ((hex >> 8) & 0xff) as u8, (hex & 0xff) as u8)
}

impl Theme {
    /// Catppuccin Mocha-like.
    pub fn dark() -> Theme {
        Theme {
            positive: rgb(0xa6e3a1),
            negative: rgb(0xf38ba8),
            warning: rgb(0xf9e2af),
            accent: rgb(0x89b4fa),
            muted: rgb(0x6c7086),
            text: rgb(0xcdd6f4),
            bg_selected: rgb(0x313244),
            chip_vacation: rgb(0x94e2d5),
            chip_flex: rgb(0xcba6f7),
            chip_holiday: rgb(0xfab387),
            chip_sick: rgb(0xf38ba8),
            chip_absence: rgb(0xf5c2e7),
            projects: [
                rgb(0x89b4fa), rgb(0xa6e3a1), rgb(0xf9e2af), rgb(0xcba6f7), rgb(0x94e2d5),
                rgb(0xfab387), rgb(0xf5c2e7), rgb(0x74c7ec), rgb(0xeba0ac), rgb(0xb4befe),
            ],
            border: BorderType::Rounded,
        }
    }

    /// Solarized-light-like.
    pub fn light() -> Theme {
        Theme {
            positive: rgb(0x859900),
            negative: rgb(0xdc322f),
            warning: rgb(0xb58900),
            accent: rgb(0x268bd2),
            muted: rgb(0x93a1a1),
            text: rgb(0x073642),
            bg_selected: rgb(0xeee8d5),
            chip_vacation: rgb(0x2aa198),
            chip_flex: rgb(0x6c71c4),
            chip_holiday: rgb(0xcb4b16),
            chip_sick: rgb(0xdc322f),
            chip_absence: rgb(0xd33682),
            projects: [
                rgb(0x268bd2), rgb(0x859900), rgb(0xb58900), rgb(0x6c71c4), rgb(0x2aa198),
                rgb(0xcb4b16), rgb(0xd33682), rgb(0x0f7fb3), rgb(0xa8332f), rgb(0x586e75),
            ],
            border: BorderType::Rounded,
        }
    }

    pub fn from_config(cfg: &Config) -> Theme {
        let mut t = if cfg.theme == "light" { Theme::light() } else { Theme::dark() };
        for (k, v) in &cfg.theme_overrides {
            let Some(c) = parse_color(v) else { continue };
            match k.as_str() {
                "positive" => t.positive = c,
                "negative" => t.negative = c,
                "warning" => t.warning = c,
                "accent" => t.accent = c,
                "muted" => t.muted = c,
                "text" => t.text = c,
                "bg_selected" => t.bg_selected = c,
                "chip_vacation" => t.chip_vacation = c,
                "chip_flex" => t.chip_flex = c,
                "chip_holiday" => t.chip_holiday = c,
                "chip_sick" => t.chip_sick = c,
                "chip_absence" => t.chip_absence = c,
                _ => {}
            }
        }
        if supports_truecolor() { t } else { t.downgrade_for_terminal() }
    }

    pub fn project_color(&self, idx: u8) -> Color {
        self.projects[(idx as usize) % self.projects.len()]
    }

    pub fn kind_color(&self, k: &DayKind) -> Color {
        match k {
            DayKind::Work => self.text,
            DayKind::Vacation => self.chip_vacation,
            DayKind::Flex => self.chip_flex,
            DayKind::Holiday => self.chip_holiday,
            DayKind::Sick => self.chip_sick,
            DayKind::Absence { .. } => self.chip_absence,
        }
    }

    pub fn minutes_style(&self, m: Minutes) -> Style {
        let fg = if m.0 > 0 { self.positive } else if m.0 < 0 { self.negative } else { self.muted };
        Style::default().fg(fg).add_modifier(Modifier::BOLD)
    }

    /// Map every Rgb color to the nearest xterm-256 index.
    pub fn downgrade_for_terminal(mut self) -> Theme {
        let f = |c: Color| match c {
            Color::Rgb(r, g, b) => Color::Indexed(nearest_256(r, g, b)),
            other => other,
        };
        self.positive = f(self.positive);
        self.negative = f(self.negative);
        self.warning = f(self.warning);
        self.accent = f(self.accent);
        self.muted = f(self.muted);
        self.text = f(self.text);
        self.bg_selected = f(self.bg_selected);
        self.chip_vacation = f(self.chip_vacation);
        self.chip_flex = f(self.chip_flex);
        self.chip_holiday = f(self.chip_holiday);
        self.chip_sick = f(self.chip_sick);
        self.chip_absence = f(self.chip_absence);
        for p in self.projects.iter_mut() {
            *p = f(*p);
        }
        self
    }
}

fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    let q = |v: u8| -> u8 { if v < 48 { 0 } else if v < 115 { 1 } else { ((v as u16 - 35) / 40) as u8 } };
    16 + 36 * q(r) + 6 * q(g) + q(b)
}

pub fn parse_color(s: &str) -> Option<Color> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#')
        && hex.len() == 6
        && let Ok(v) = u32::from_str_radix(hex, 16)
    {
        return Some(rgb(v));
    }
    Color::from_str(s).ok()
}

pub fn supports_truecolor() -> bool {
    matches!(std::env::var("COLORTERM").as_deref(), Ok("truecolor") | Ok("24bit"))
}
```

- [ ] **Step 4: Implement `src/tui/view/mod.rs`**

```rust
pub mod chrome;
pub mod day;
pub mod month;
pub mod stats;

use tuirealm::ratatui::style::{Modifier, Style};
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Block, Borders};

use super::theme::Theme;
use crate::core::Minutes;
use tuirealm::ratatui::style::Color;

pub fn minutes_span(m: Minutes, theme: &Theme) -> Span<'static> {
    Span::styled(m.to_string(), theme.minutes_style(m))
}

pub fn chip(text: &str, color: Color) -> Span<'static> {
    Span::styled(
        format!(" {text} "),
        Style::default().fg(Color::Black).bg(color).add_modifier(Modifier::BOLD),
    )
}

pub fn bar(frac: f64, width: u16, color: Color, theme: &Theme) -> Line<'static> {
    let filled = ((frac.clamp(0.0, 1.0)) * width as f64).round() as usize;
    let empty = (width as usize).saturating_sub(filled);
    Line::from(vec![
        Span::styled("█".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(empty), Style::default().fg(theme.muted)),
    ])
}

pub fn block(theme: &Theme, title: Option<&str>) -> Block<'static> {
    let mut b = Block::default()
        .borders(Borders::ALL)
        .border_type(theme.border)
        .border_style(Style::default().fg(theme.muted));
    if let Some(t) = title {
        b = b.title(Span::styled(format!(" {t} "), Style::default().fg(theme.text).add_modifier(Modifier::BOLD)));
    }
    b
}

// + the `testing` module from the architecture notes above
```

Create empty stubs `src/tui/view/{chrome,day,month,stats}.rs` containing `//! filled in later` so the crate compiles.

- [ ] **Step 5: Run tests, check, commit**

Run: `nix develop -c cargo test tui::theme` → 5 passed. `./scripts/check.sh`.

```bash
git add src/tui
git commit -m "feat(tui): theme with overrides, 256-color fallback, drawing helpers"
```

---

### Task 12: TUI runtime skeleton (tokio, worker, chrome, quit)

**Files:**
- Create: `src/tui/ids.rs`, `src/tui/msg.rs`, `src/tui/worker.rs`, `src/tui/model.rs`, `src/tui/components/mod.rs`, `src/tui/components/bridge.rs`, `src/tui/components/month.rs`, `src/tui/view/chrome.rs`
- Modify: `src/tui/mod.rs`

**Interfaces:**
- Consumes: `Ctx` from cli, `Theme`, store types.
- Produces:
  ```rust
  // ids.rs
  #[derive(Debug, Eq, PartialEq, Clone, Hash)] pub enum Id { Bridge, Month, Day, Form, Stats, Confirm, Help }
  // msg.rs
  #[derive(Debug, PartialEq, Clone)] pub enum Msg {
      Quit, Tick, Store(StoreReply), Error(String), Info(String),
      SelectDay(i32 /* +-days */), SelectMonth(i32), GoToday, OpenDay, OpenStats, Back,
      ClockIn, ClockOut, SetKind(DayKind), AskConfirm(Confirm), ConfirmYes, ConfirmNo, ToggleHelp,
      // day editor
      DaySelect(i32), DayAdd, DayEdit, DayDelete, DayKindNext, DayKindPrev,
      // form
      FormSubmit(FormData), FormCancel,
      // stats
      StatsRange(RangeKind),
  }
  #[derive(Debug, PartialEq, Clone)] pub enum Confirm { DeleteEntry(i64), SetKind(NaiveDate, DayKind), ClockInReplace }
  #[derive(Debug, PartialEq, Clone)] pub struct FormData { pub id: Option<i64>, pub start: String, pub end: String, pub project: String, pub comment: String }
  #[derive(Debug, PartialEq, Clone, Copy)] pub enum RangeKind { ThisMonth, LastMonth, Quarter, Year }
  pub enum StoreCmd { LoadMonth { year: i32, month: u32 }, LoadDay(NaiveDate), LoadStats { from: NaiveDate, to: NaiveDate },
      AddEntry { date, start, end, project, comment }, UpdateEntry { id, start, end, project, comment }, DeleteEntry(i64),
      SetKind(NaiveDate, DayKind), ClockIn(NaiveDate, NaiveTime), ClockOut { project: Option<String>, comment: String }, Shutdown }
  #[derive(Debug, PartialEq, Clone)] pub enum StoreReply {
      Month(MonthData), Day(DayData), Stats(StatsData), Changed, Failed(String) }
  #[derive(Debug, PartialEq, Clone)] pub struct MonthData { pub year: i32, pub month: u32, pub days: Vec<Day>, pub balance_before: Minutes /* running balance up to last day of previous month */, pub session: Option<Session>, pub projects: Vec<Project>, pub vacation_used_this_year: u32, pub balance_total: Minutes }
  #[derive(Debug, PartialEq, Clone)] pub struct DayData { pub day: Day, pub projects: Vec<Project> }
  #[derive(Debug, PartialEq, Clone)] pub struct StatsData { pub from: NaiveDate, pub to: NaiveDate, pub days: Vec<Day>, pub projects: Vec<Project> }
  #[derive(Debug, Clone)] pub enum UserEvent { Store(StoreReply) }  // PartialEq/Eq by discriminant
  // worker.rs
  pub struct Worker { pub tx: std::sync::mpsc::Sender<StoreCmd> }
  pub fn spawn_worker(ctx: Ctx, reply_tx: tokio::sync::mpsc::UnboundedSender<StoreReply>) -> Worker  // std thread owning Store+Config
  pub struct StorePort { rx: tokio::sync::mpsc::UnboundedReceiver<StoreReply> }  // impl PollAsync<UserEvent>
  // model.rs
  pub enum Screen { Month, Day, Stats }
  pub struct Model { pub app: Application<Id, Msg, UserEvent>, pub terminal: Option<CrosstermTerminalAdapter>, pub theme: Theme, pub rules: Rules, pub cal: HolidayCalendar,
      pub screen: Screen, pub selected: NaiveDate, pub today: NaiveDate, pub now: NaiveTime, pub month: Option<MonthData>, pub day: Option<DayData>, pub stats: Option<StatsData>,
      pub day_cursor: usize, pub confirm: Option<Confirm>, pub help: bool, pub status: Option<(String, bool /*is_error*/, Instant)>, pub quit: bool, pub redraw: bool, pub worker: Worker, pub size: (u16,u16) }
  impl Model { pub fn update(&mut self, msg: Msg); pub fn view(&mut self); pub fn draw(&self, f: &mut Frame) /* pure over &self */ }
  // mod.rs
  pub fn run(ctx: Ctx) -> anyhow::Result<()>
  ```

- [ ] **Step 1: Write the failing test for chrome drawing (`src/tui/view/chrome.rs`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme::Theme;
    use crate::tui::view::testing::{contains, render};
    use crate::core::Minutes;

    #[test]
    fn title_bar_shows_month_balance_and_clock() {
        let t = Theme::dark();
        let rows = render(100, 1, |f| {
            draw_title_bar(f, f.area(), &t, &TitleInfo {
                title: "SEPTEMBER 2026".into(), balance: Minutes(750),
                clock: Some(("08:12".into(), Minutes(221))), vacation: Some((21, 30)),
            });
        });
        assert!(contains(&rows, "SEPTEMBER 2026"));
        assert!(contains(&rows, "+12:30"));
        assert!(contains(&rows, "in since 08:12"));
        assert!(contains(&rows, "03:41"));
        assert!(contains(&rows, "21/30"));
    }

    #[test]
    fn key_hints_and_status() {
        let t = Theme::dark();
        let rows = render(100, 2, |f| {
            let [a, b] = tuirealm::ratatui::layout::Layout::vertical([
                tuirealm::ratatui::layout::Constraint::Length(1),
                tuirealm::ratatui::layout::Constraint::Length(1),
            ]).areas(f.area());
            draw_status_bar(f, a, &t, Some(("saved", false)));
            draw_key_hints(f, b, &t, &[("↑↓", "day"), ("q", "quit")]);
        });
        assert!(contains(&rows, "saved"));
        assert!(contains(&rows, "↑↓ day"));
        assert!(contains(&rows, "q quit"));
    }

    #[test]
    fn too_small_notice() {
        let t = Theme::dark();
        let rows = render(40, 10, |f| draw_too_small(f, f.area(), &t, (40, 10)));
        assert!(contains(&rows, "80"));
        assert!(contains(&rows, "24"));
    }
}
```

- [ ] **Step 2: Implement `src/tui/view/chrome.rs`**

```rust
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Alignment, Constraint, Layout, Rect};
use tuirealm::ratatui::style::{Modifier, Style};
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Clear, Paragraph, Wrap};

use super::{block, minutes_span};
use crate::core::Minutes;
use crate::tui::theme::Theme;

pub struct TitleInfo {
    pub title: String,
    pub balance: Minutes,
    /// (clock-in time "HH:MM", running minutes)
    pub clock: Option<(String, Minutes)>,
    /// (used, allowance)
    pub vacation: Option<(u32, u32)>,
}

pub fn draw_title_bar(f: &mut Frame, area: Rect, t: &Theme, info: &TitleInfo) {
    let mut right: Vec<Span> = vec![Span::styled("balance ", Style::default().fg(t.muted)), minutes_span(info.balance, t)];
    if let Some((since, running)) = &info.clock {
        right.push(Span::raw("   "));
        right.push(Span::styled(format!("⏱ in since {since} ({})", running.hhmm()), Style::default().fg(t.warning)));
    }
    if let Some((used, allow)) = info.vacation {
        right.push(Span::raw("   "));
        right.push(Span::styled(format!("vacation {used}/{allow}"), Style::default().fg(t.muted)));
    }
    let [l, r] = Layout::horizontal([Constraint::Min(20), Constraint::Length(70)]).areas(area);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(format!(" {}", info.title), Style::default().fg(t.accent).add_modifier(Modifier::BOLD)))),
        l,
    );
    f.render_widget(Paragraph::new(Line::from(right)).alignment(Alignment::Right), r);
}

pub fn draw_status_bar(f: &mut Frame, area: Rect, t: &Theme, status: Option<(&str, bool)>) {
    let line = match status {
        Some((msg, true)) => Line::from(Span::styled(format!(" ✖ {msg}"), Style::default().fg(t.negative))),
        Some((msg, false)) => Line::from(Span::styled(format!(" ✔ {msg}"), Style::default().fg(t.positive))),
        None => Line::from(""),
    };
    f.render_widget(Paragraph::new(line), area);
}

pub fn draw_key_hints(f: &mut Frame, area: Rect, t: &Theme, hints: &[(&str, &str)]) {
    let mut spans = vec![Span::raw(" ")];
    for (i, (k, d)) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", Style::default()));
        }
        spans.push(Span::styled(*k, Style::default().fg(t.accent).add_modifier(Modifier::BOLD)));
        spans.push(Span::styled(format!(" {d}"), Style::default().fg(t.muted)));
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

pub fn draw_too_small(f: &mut Frame, area: Rect, t: &Theme, size: (u16, u16)) {
    let text = format!("Terminal too small: {}×{}. Need at least 80×24.", size.0, size.1);
    f.render_widget(
        Paragraph::new(Span::styled(text, Style::default().fg(t.warning))).alignment(Alignment::Center).wrap(Wrap { trim: true }),
        area,
    );
}

pub fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

pub fn draw_confirm(f: &mut Frame, area: Rect, t: &Theme, question: &str) {
    let r = centered(area, (question.len() as u16 + 6).max(30), 5);
    f.render_widget(Clear, r);
    let p = Paragraph::new(vec![
        Line::from(question),
        Line::from(""),
        Line::from(vec![
            Span::styled("y", Style::default().fg(t.accent).add_modifier(Modifier::BOLD)), Span::styled(" yes   ", Style::default().fg(t.muted)),
            Span::styled("n", Style::default().fg(t.accent).add_modifier(Modifier::BOLD)), Span::styled(" no", Style::default().fg(t.muted)),
        ]),
    ])
    .alignment(Alignment::Center)
    .block(block(t, Some("Confirm")));
    f.render_widget(p, r);
}

pub fn draw_help(f: &mut Frame, area: Rect, t: &Theme, title: &str, keys: &[(&str, &str)]) {
    let h = (keys.len() as u16 + 2).min(area.height);
    let r = centered(area, 50, h);
    f.render_widget(Clear, r);
    let lines: Vec<Line> = keys
        .iter()
        .map(|(k, d)| Line::from(vec![Span::styled(format!("{k:>10}  "), Style::default().fg(t.accent).add_modifier(Modifier::BOLD)), Span::raw(*d)]))
        .collect();
    f.render_widget(Paragraph::new(lines).block(block(t, Some(title))), r);
}
```

- [ ] **Step 3: Implement `src/tui/ids.rs` and `src/tui/msg.rs`**

`ids.rs`:
```rust
#[derive(Debug, Eq, PartialEq, Clone, Hash)]
pub enum Id {
    Bridge,
    Month,
    Day,
    Form,
    Stats,
    Confirm,
    Help,
}
```

`msg.rs`: the enums from the Interfaces block above, with full field lists:
```rust
use chrono::{NaiveDate, NaiveTime};

use crate::core::{Day, DayKind, Minutes, Project};
use crate::store::Session;

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum RangeKind { ThisMonth, LastMonth, Quarter, Year }

#[derive(Debug, PartialEq, Clone)]
pub enum Confirm {
    DeleteEntry(i64),
    SetKind(NaiveDate, DayKind),
    ClockInReplace,
}

#[derive(Debug, PartialEq, Clone, Default)]
pub struct FormData {
    pub id: Option<i64>,
    pub start: String,
    pub end: String,
    pub project: String,
    pub comment: String,
}

#[derive(Debug, PartialEq, Clone)]
pub enum Msg {
    Quit, Tick, Store(StoreReply), Error(String), Info(String),
    SelectDay(i32), SelectMonth(i32), GoToday, OpenDay, OpenStats, Back,
    ClockIn, ClockOut, SetKind(DayKind), AskConfirm(Confirm), ConfirmYes, ConfirmNo, ToggleHelp,
    DaySelect(i32), DayAdd, DayEdit, DayDelete, DayKindNext, DayKindPrev,
    FormSubmit(FormData), FormCancel,
    StatsRange(RangeKind),
}

#[derive(Debug, Clone)]
pub enum StoreCmd {
    LoadMonth { year: i32, month: u32 },
    LoadDay(NaiveDate),
    LoadStats { from: NaiveDate, to: NaiveDate },
    AddEntry { date: NaiveDate, start: NaiveTime, end: NaiveTime, project: String, comment: String },
    UpdateEntry { id: i64, start: NaiveTime, end: NaiveTime, project: String, comment: String },
    DeleteEntry(i64),
    SetKind(NaiveDate, DayKind),
    ClockIn(NaiveDate, NaiveTime),
    ClockOut { project: Option<String>, comment: String },
    Shutdown,
}

#[derive(Debug, PartialEq, Clone)]
pub struct MonthData {
    pub year: i32,
    pub month: u32,
    pub days: Vec<Day>,
    pub balance_before: Minutes,
    pub balance_total: Minutes,
    pub session: Option<Session>,
    pub projects: Vec<Project>,
    pub vacation_used_this_year: u32,
}

#[derive(Debug, PartialEq, Clone)]
pub struct DayData { pub day: Day, pub projects: Vec<Project> }

#[derive(Debug, PartialEq, Clone)]
pub struct StatsData { pub from: NaiveDate, pub to: NaiveDate, pub days: Vec<Day>, pub projects: Vec<Project> }

#[derive(Debug, PartialEq, Clone)]
pub enum StoreReply {
    Month(MonthData),
    Day(DayData),
    Stats(StatsData),
    Changed(String),   // success message
    Failed(String),
}

#[derive(Debug, Clone)]
pub enum UserEvent { Store(StoreReply) }

impl PartialEq for UserEvent {
    fn eq(&self, other: &Self) -> bool { std::mem::discriminant(self) == std::mem::discriminant(other) }
}
impl Eq for UserEvent {}
```

- [ ] **Step 4: Implement `src/tui/worker.rs`**

```rust
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

use chrono::{Datelike, Days, Local, NaiveDate, Timelike};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tuirealm::event::Event;
use tuirealm::listener::{PollAsync, PortResult};

use super::msg::{DayData, MonthData, StatsData, StoreCmd, StoreReply, UserEvent};
use crate::cli::Ctx;
use crate::core::{DayKind, TodayCtx, day_stats, running_balance};

pub struct Worker { pub tx: Sender<StoreCmd> }

impl Worker {
    pub fn send(&self, cmd: StoreCmd) { let _ = self.tx.send(cmd); }
}

pub fn spawn_worker(ctx: Ctx, reply_tx: UnboundedSender<StoreReply>) -> Worker {
    let (tx, rx) = channel::<StoreCmd>();
    thread::Builder::new().name("tk-store".into()).spawn(move || worker_loop(ctx, rx, reply_tx)).expect("spawn worker");
    Worker { tx }
}

fn worker_loop(ctx: Ctx, rx: Receiver<StoreCmd>, reply: UnboundedSender<StoreReply>) {
    while let Ok(cmd) = rx.recv() {
        if matches!(cmd, StoreCmd::Shutdown) { break; }
        let r = handle(&ctx, cmd).unwrap_or_else(|e| StoreReply::Failed(e.to_string()));
        if reply.send(r).is_err() { break; }
    }
}

pub fn month_range(year: i32, month: u32) -> (NaiveDate, NaiveDate) {
    let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    let next = if month == 12 { NaiveDate::from_ymd_opt(year + 1, 1, 1) } else { NaiveDate::from_ymd_opt(year, month + 1, 1) }.unwrap();
    (first, next.pred_opt().unwrap())
}

fn balance_through(ctx: &Ctx, to: NaiveDate, today: NaiveDate, clocked_in: bool) -> anyhow::Result<crate::core::Minutes> {
    let rules = ctx.config.rules();
    let cal = ctx.config.calendar();
    if to < rules.start_date { return Ok(rules.initial_balance); }
    let days = ctx.store.days_in(rules.start_date, to, &cal)?;
    let tc = TodayCtx { today, clocked_in };
    let stats: Vec<_> = days.iter().map(|d| day_stats(d, &rules, &cal, &tc)).collect();
    Ok(running_balance(&stats, &rules))
}

fn handle(ctx: &Ctx, cmd: StoreCmd) -> anyhow::Result<StoreReply> {
    let now = Local::now().naive_local();
    let today = now.date();
    Ok(match cmd {
        StoreCmd::LoadMonth { year, month } => {
            let (from, to) = month_range(year, month);
            let cal = ctx.config.calendar();
            let session = ctx.store.session()?;
            let days = ctx.store.days_in(from, to, &cal)?;
            let (y0, y1) = (NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap(), NaiveDate::from_ymd_opt(today.year(), 12, 31).unwrap());
            let vacation_used_this_year = ctx.store.stored_kinds_in(y0, y1)?.iter()
                .filter(|(d, k)| **k == DayKind::Vacation && crate::core::is_working_day(**d)).count() as u32;
            StoreReply::Month(MonthData {
                year, month, days,
                balance_before: balance_through(ctx, from.pred_opt().unwrap(), today, session.is_some())?,
                balance_total: balance_through(ctx, today.max(to.min(today)), today, session.is_some())?,
                session,
                projects: ctx.store.list_projects(false)?,
                vacation_used_this_year,
            })
        }
        StoreCmd::LoadDay(date) => StoreReply::Day(DayData {
            day: ctx.store.days_in(date, date, &ctx.config.calendar())?.remove(0),
            projects: ctx.store.list_projects(false)?,
        }),
        StoreCmd::LoadStats { from, to } => StoreReply::Stats(StatsData {
            from, to, days: ctx.store.days_in(from, to, &ctx.config.calendar())?, projects: ctx.store.list_projects(true)?,
        }),
        StoreCmd::AddEntry { date, start, end, project, comment } => {
            let e = ctx.store.add_entry(date, start, end, &project, &comment)?;
            StoreReply::Changed(format!("Added {}–{} {}", e.start.format("%H:%M"), e.end.format("%H:%M"), e.project))
        }
        StoreCmd::UpdateEntry { id, start, end, project, comment } => {
            let e = ctx.store.update_entry(id, start, end, &project, &comment)?;
            StoreReply::Changed(format!("Updated {}–{} {}", e.start.format("%H:%M"), e.end.format("%H:%M"), e.project))
        }
        StoreCmd::DeleteEntry(id) => { ctx.store.delete_entry(id)?; StoreReply::Changed("Entry deleted".into()) }
        StoreCmd::SetKind(date, kind) => { ctx.store.set_day_kind(date, &kind)?; StoreReply::Changed(format!("{date} set to {}", kind.display_name().to_lowercase())) }
        StoreCmd::ClockIn(date, t) => { ctx.store.clock_in(date, t)?; StoreReply::Changed(format!("Clocked in at {}", t.format("%H:%M"))) }
        StoreCmd::ClockOut { project, comment } => {
            let s = ctx.store.session()?.ok_or_else(|| anyhow::anyhow!("not clocked in"))?;
            let project = match project.or(ctx.store.last_used_project()?) {
                Some(p) => p,
                None => anyhow::bail!("no project yet; add the entry from the day editor"),
            };
            let end = now.time().with_second(0).unwrap();
            let e = ctx.store.add_entry(s.date, s.start, end, &project, &comment)?;
            ctx.store.clear_session()?;
            StoreReply::Changed(format!("Clocked out: {}–{} {} ({})", e.start.format("%H:%M"), e.end.format("%H:%M"), e.project, e.duration()))
        }
        StoreCmd::Shutdown => StoreReply::Changed(String::new()),
    })
}

pub struct StorePort { rx: UnboundedReceiver<StoreReply> }

impl StorePort {
    pub fn new(rx: UnboundedReceiver<StoreReply>) -> Self { Self { rx } }
}

#[tuirealm::async_trait]
impl PollAsync<UserEvent> for StorePort {
    async fn poll(&mut self) -> PortResult<Option<Event<UserEvent>>> {
        Ok(self.rx.recv().await.map(|r| Event::User(UserEvent::Store(r))))
    }
}
```

Unused `Days` import: remove if clippy complains. `balance_total` is the running balance through today (or through the loaded month's end if that is earlier, which for past months still ends today — simplify to `balance_through(ctx, today, ...)`).

- [ ] **Step 5: Implement components `bridge.rs` and `month.rs` (`src/tui/components/`)**

`components/mod.rs`:
```rust
pub mod bridge;
pub mod day;
pub mod form;
pub mod month;
pub mod stats;

use tuirealm::command::{Cmd, CmdResult};
use tuirealm::component::Component;
use tuirealm::props::{AttrValue, Attribute, Props, QueryResult};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Rect;
use tuirealm::state::State;

/// A component that draws nothing and only routes keys. All drawing is done by `Model::draw`.
#[derive(Default)]
pub struct KeyOnly { props: Props }

impl Component for KeyOnly {
    fn view(&mut self, _f: &mut Frame, _a: Rect) {}
    fn query<'a>(&'a self, attr: Attribute) -> Option<QueryResult<'a>> { self.props.get_for_query(attr) }
    fn attr(&mut self, attr: Attribute, value: AttrValue) { self.props.set(attr, value); }
    fn state(&self) -> State { State::None }
    fn perform(&mut self, cmd: Cmd) -> CmdResult { CmdResult::Invalid(cmd) }
}
```

`components/bridge.rs` — converts Tick and store replies into Msgs:
```rust
use tuirealm::component::AppComponent;
use tuirealm::event::Event;
use tuirealm::{Component as _};  // not needed if derive works; see below

use super::KeyOnly;
use crate::tui::msg::{Msg, UserEvent};

#[derive(Default, tuirealm::Component)]
pub struct Bridge { component: KeyOnly }

impl AppComponent<Msg, UserEvent> for Bridge {
    fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg> {
        match ev {
            Event::Tick => Some(Msg::Tick),
            Event::User(UserEvent::Store(r)) => Some(Msg::Store(r.clone())),
            _ => None,
        }
    }
}
```
(The `#[derive(Component)]` macro from `tuirealm_derive` expects a field named `component` implementing `Component`; remove the stray `use tuirealm::{Component as _}` line if it does not compile.)

`components/month.rs` — month screen key map:
```rust
use tuirealm::component::AppComponent;
use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers};

use super::KeyOnly;
use crate::core::DayKind;
use crate::tui::msg::{Msg, UserEvent};

#[derive(Default, tuirealm::Component)]
pub struct MonthScreen { component: KeyOnly }

impl AppComponent<Msg, UserEvent> for MonthScreen {
    fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg> {
        let KeyEvent { code, modifiers } = ev.as_keyboard()?;
        if *modifiers == KeyModifiers::CONTROL && *code == Key::Char('c') { return Some(Msg::Quit); }
        Some(match code {
            Key::Char('q') => Msg::Quit,
            Key::Up | Key::Char('k') => Msg::SelectDay(-1),
            Key::Down | Key::Char('j') => Msg::SelectDay(1),
            Key::Char('[') | Key::PageUp => Msg::SelectMonth(-1),
            Key::Char(']') | Key::PageDown => Msg::SelectMonth(1),
            Key::Char('t') => Msg::GoToday,
            Key::Enter => Msg::OpenDay,
            Key::Char('s') => Msg::OpenStats,
            Key::Char('i') => Msg::ClockIn,
            Key::Char('o') => Msg::ClockOut,
            Key::Char('v') => Msg::SetKind(DayKind::Vacation),
            Key::Char('f') => Msg::SetKind(DayKind::Flex),
            Key::Char('x') => Msg::SetKind(DayKind::Sick),
            Key::Char('p') => Msg::SetKind(DayKind::Holiday),
            Key::Char('w') => Msg::SetKind(DayKind::Work),
            Key::Char('?') => Msg::ToggleHelp,
            Key::Esc => Msg::Back,
            _ => return None,
        })
    }
}
```

Create `components/day.rs`, `components/form.rs`, `components/stats.rs` as stubs containing only `//! Task 15/16/18` so `mod` lines compile.

- [ ] **Step 6: Implement `src/tui/model.rs` (skeleton; screens fill in later tasks)**

```rust
use std::time::{Duration, Instant};

use chrono::{Datelike, Days, Local, NaiveDate, NaiveTime};
use tuirealm::application::Application;
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout};
use tuirealm::terminal::{CrosstermTerminalAdapter, TerminalAdapter};

use super::ids::Id;
use super::msg::{Confirm, DayData, MonthData, Msg, StatsData, StoreCmd, StoreReply, UserEvent};
use super::theme::Theme;
use super::view::chrome;
use super::worker::Worker;
use crate::core::{HolidayCalendar, Rules};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen { Month, Day, Stats }

pub struct Model {
    pub app: Application<Id, Msg, UserEvent>,
    pub terminal: Option<CrosstermTerminalAdapter>,
    pub theme: Theme,
    pub rules: Rules,
    pub cal: HolidayCalendar,
    pub vacation_allowance: u32,
    pub screen: Screen,
    pub selected: NaiveDate,
    pub today: NaiveDate,
    pub now: NaiveTime,
    pub month: Option<MonthData>,
    pub day: Option<DayData>,
    pub stats: Option<StatsData>,
    pub day_cursor: usize,
    pub confirm: Option<Confirm>,
    pub help: bool,
    pub status: Option<(String, bool, Instant)>,
    pub quit: bool,
    pub redraw: bool,
    pub worker: Worker,
    pub size: (u16, u16),
}

pub const STATUS_TTL: Duration = Duration::from_secs(5);

impl Model {
    pub fn set_status(&mut self, msg: impl Into<String>, is_error: bool) {
        self.status = Some((msg.into(), is_error, Instant::now()));
    }

    pub fn load_month(&self) {
        self.worker.send(StoreCmd::LoadMonth { year: self.selected.year(), month: self.selected.month() });
    }

    pub fn update(&mut self, msg: Msg) {
        self.redraw = true;
        match msg {
            Msg::Quit => self.quit = true,
            Msg::Tick => {
                let now = Local::now().naive_local();
                if now.date() != self.today {
                    self.today = now.date();
                    self.load_month();
                }
                self.now = now.time();
                if let Some((_, _, at)) = &self.status && at.elapsed() > STATUS_TTL { self.status = None; }
            }
            Msg::Error(e) => self.set_status(e, true),
            Msg::Info(i) => self.set_status(i, false),
            Msg::Store(reply) => self.on_store(reply),
            Msg::SelectDay(n) => {
                let d = if n >= 0 { self.selected.checked_add_days(Days::new(n as u64)) } else { self.selected.checked_sub_days(Days::new((-n) as u64)) };
                if let Some(d) = d {
                    let changed_month = (d.year(), d.month()) != (self.selected.year(), self.selected.month());
                    self.selected = d;
                    if changed_month { self.load_month(); }
                }
            }
            Msg::SelectMonth(n) => {
                let (y, m) = (self.selected.year(), self.selected.month() as i32 - 1 + n);
                let (y, m) = (y + m.div_euclid(12), (m.rem_euclid(12) + 1) as u32);
                let last = super::worker::month_range(y, m).1;
                self.selected = NaiveDate::from_ymd_opt(y, m, self.selected.day().min(last.day())).unwrap();
                self.load_month();
            }
            Msg::GoToday => { self.selected = self.today; self.load_month(); }
            Msg::ToggleHelp => self.help = !self.help,
            Msg::Back => { if self.help { self.help = false } else if self.confirm.is_some() { self.confirm = None } else { self.screen = Screen::Month; let _ = self.app.active(&Id::Month); } }
            // Filled in by Tasks 15–18:
            _ => {}
        }
    }

    fn on_store(&mut self, reply: StoreReply) {
        match reply {
            StoreReply::Month(m) => self.month = Some(m),
            StoreReply::Day(d) => self.day = Some(d),
            StoreReply::Stats(s) => self.stats = Some(s),
            StoreReply::Changed(msg) => { if !msg.is_empty() { self.set_status(msg, false); } self.load_month(); if self.screen == Screen::Day { self.worker.send(StoreCmd::LoadDay(self.selected)); } }
            StoreReply::Failed(e) => self.set_status(e, true),
        }
    }

    pub fn view(&mut self) {
        let mut term = self.terminal.take().expect("terminal");
        let _ = term.draw(|f| self.draw(f));
        self.terminal = Some(term);
    }

    /// Pure over &self: used by snapshot tests through a TestBackend.
    pub fn draw(&self, f: &mut Frame) {
        let area = f.area();
        if area.width < 80 || area.height < 24 {
            chrome::draw_too_small(f, area, &self.theme, (area.width, area.height));
            return;
        }
        let [title, body, status, hints] = Layout::vertical([
            Constraint::Length(1), Constraint::Min(5), Constraint::Length(1), Constraint::Length(1),
        ]).areas(area);
        chrome::draw_title_bar(f, title, &self.theme, &self.title_info());
        match self.screen {
            Screen::Month => super::view::month::draw(self, f, body),
            Screen::Day => super::view::day::draw(self, f, body),
            Screen::Stats => super::view::stats::draw(self, f, body),
        }
        chrome::draw_status_bar(f, status, &self.theme, self.status.as_ref().map(|(m, e, _)| (m.as_str(), *e)));
        chrome::draw_key_hints(f, hints, &self.theme, self.key_hints());
        if let Some(c) = &self.confirm { chrome::draw_confirm(f, area, &self.theme, &self.confirm_text(c)); }
        if self.help { chrome::draw_help(f, area, &self.theme, "Keys", self.help_keys()); }
    }

    pub fn title_info(&self) -> chrome::TitleInfo {
        let m = self.month.as_ref();
        chrome::TitleInfo {
            title: format!("{} {}", month_name(self.selected.month()).to_uppercase(), self.selected.year()),
            balance: m.map(|m| m.balance_total).unwrap_or_default(),
            clock: m.and_then(|m| m.session.as_ref()).map(|s| (s.start.format("%H:%M").to_string(), crate::core::Minutes(((self.now - s.start).num_minutes().max(0)) as i32))),
            vacation: m.map(|m| (m.vacation_used_this_year, self.vacation_allowance)),
        }
    }

    pub fn key_hints(&self) -> &'static [(&'static str, &'static str)] {
        match self.screen {
            Screen::Month => &[("↑↓", "day"), ("[ ]", "month"), ("⏎", "edit"), ("i/o", "clock"), ("s", "stats"), ("v f x p", "day type"), ("?", "help"), ("q", "quit")],
            Screen::Day => &[("↑↓", "entry"), ("a", "add"), ("e", "edit"), ("d", "delete"), ("←→", "day type"), ("Esc", "back")],
            Screen::Stats => &[("1", "month"), ("2", "last month"), ("3", "quarter"), ("4", "year"), ("Esc", "back")],
        }
    }

    pub fn help_keys(&self) -> &'static [(&'static str, &'static str)] {
        match self.screen {
            Screen::Month => &[("↑ ↓ j k", "move by day"), ("[ ]", "previous / next month"), ("t", "jump to today"), ("Enter", "open day editor"), ("s", "statistics"), ("i / o", "clock in / out"), ("v", "vacation"), ("f", "flex day"), ("x", "sick"), ("p", "public holiday"), ("w", "reset to work day"), ("q", "quit")],
            Screen::Day => &[("↑ ↓", "select entry"), ("a", "add entry"), ("e", "edit entry"), ("d", "delete entry"), ("← →", "change day type"), ("Esc", "back")],
            Screen::Stats => &[("1 2 3 4", "range"), ("Esc", "back")],
        }
    }

    pub fn confirm_text(&self, c: &Confirm) -> String {
        match c {
            Confirm::DeleteEntry(_) => "Delete this entry?".into(),
            Confirm::SetKind(d, k) => format!("Set {d} to {}?", k.display_name().to_lowercase()),
            Confirm::ClockInReplace => "Already clocked in. Replace?".into(),
        }
    }
}

pub fn month_name(m: u32) -> &'static str {
    ["January","February","March","April","May","June","July","August","September","October","November","December"][(m as usize).saturating_sub(1).min(11)]
}
```

Stub `view::month::draw`, `view::day::draw`, `view::stats::draw` as `pub fn draw(_m: &Model, _f: &mut Frame, _area: Rect) {}` for now.

- [ ] **Step 7: Implement `src/tui/mod.rs`**

```rust
pub mod components;
pub mod ids;
pub mod model;
pub mod msg;
pub mod theme;
pub mod view;
pub mod worker;

use std::time::Duration;

use chrono::Local;
use tuirealm::application::{Application, PollStrategy};
use tuirealm::listener::EventListenerCfg;
use tuirealm::subscription::{EventClause, Sub, SubClause};
use tuirealm::terminal::{CrosstermTerminalAdapter, TerminalAdapter};

use crate::cli::Ctx;
use ids::Id;
use model::{Model, Screen};
use msg::{Msg, StoreReply, UserEvent};

pub fn run(ctx: Ctx) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().worker_threads(2).build()?;
    let _guard = rt.enter();

    let theme = theme::Theme::from_config(&ctx.config);
    let rules = ctx.config.rules();
    let cal = ctx.config.calendar();
    let vacation_allowance = ctx.config.vacation_days_per_year;

    let (reply_tx, reply_rx) = tokio::sync::mpsc::unbounded_channel::<StoreReply>();
    let worker = worker::spawn_worker(ctx, reply_tx);

    let cfg = EventListenerCfg::default()
        .with_handle(rt.handle().clone())
        .async_crossterm_input_listener(Duration::ZERO, 3)
        .add_async_port(Box::new(worker::StorePort::new(reply_rx)), Duration::ZERO, 1)
        .tick_interval(Duration::from_secs(1));
    let mut app: Application<Id, Msg, UserEvent> = Application::init(cfg);
    app.mount(
        Id::Bridge,
        Box::new(components::bridge::Bridge::default()),
        vec![
            Sub::new(EventClause::Tick, SubClause::Always),
            Sub::new(EventClause::User(UserEvent::Store(StoreReply::Changed(String::new()))), SubClause::Always),
        ],
    )?;
    app.mount(Id::Month, Box::new(components::month::MonthScreen::default()), vec![])?;
    app.active(&Id::Month)?;

    // Restore the terminal on panic so the shell is usable.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm_restore();
        default_hook(info);
    }));

    let mut terminal = CrosstermTerminalAdapter::new()?;
    terminal.enable_raw_mode()?;
    terminal.enter_alternate_screen()?;

    let now = Local::now().naive_local();
    let mut model = Model {
        app, terminal: Some(terminal), theme, rules, cal, vacation_allowance,
        screen: Screen::Month, selected: now.date(), today: now.date(), now: now.time(),
        month: None, day: None, stats: None, day_cursor: 0, confirm: None, help: false,
        status: None, quit: false, redraw: true, worker, size: (0, 0),
    };
    model.load_month();

    while !model.quit {
        match model.app.tick(PollStrategy::Once(Duration::from_millis(10))) {
            Err(e) => model.set_status(format!("event error: {e}"), true),
            Ok(msgs) => for m in msgs { model.update(m); },
        }
        if model.redraw { model.view(); model.redraw = false; }
    }
    model.worker.send(msg::StoreCmd::Shutdown);
    if let Some(mut t) = model.terminal.take() { t.restore()?; }
    Ok(())
}

fn crossterm_restore() -> std::io::Result<()> {
    use tuirealm::ratatui::crossterm::{execute, terminal};
    terminal::disable_raw_mode()?;
    execute!(std::io::stdout(), terminal::LeaveAlternateScreen)
}
```

If `tuirealm::ratatui::crossterm` is not re-exported, add `crossterm = "0.29"` to `Cargo.toml` and use it directly.

- [ ] **Step 8: Build, run tests, manual smoke test**

Run: `nix develop -c cargo test tui` → chrome tests pass (3).
Run: `TK_HOME=/tmp/tk-smoke nix develop -c cargo run` in a real terminal: title bar shows the current month and `balance +00:00`, pressing `q` exits cleanly and the shell is intact. `[` and `]` change the title month.

- [ ] **Step 9: Check and commit**

Run: `./scripts/check.sh`

```bash
git add src/tui
git commit -m "feat(tui): tui-realm runtime with tokio store worker, ticker, chrome"
```

---

### Task 13: Month view (table rows, week footers, snapshot tests)

**Files:**
- Create: `src/tui/view/month.rs` (replace stub)

**Interfaces:**
- Consumes: `MonthData`, `Rules`, `HolidayCalendar`, `TodayCtx`, `day_stats`, `Theme`, helpers from `view/mod.rs`.
- Produces:
  ```rust
  pub enum RowKind { Entry { entry_idx: usize, first: bool }, Missing, Empty, Kind, Weekend, WeekFooter { week: u32, week_balance: Minutes, running: Minutes } }
  pub struct Row { pub date: NaiveDate, pub kind: RowKind, pub stats: Option<DayStats> }
  pub struct MonthView { pub rows: Vec<Row>, pub target: Minutes, pub net: Minutes, pub balance: Minutes, pub project_totals: Vec<(String, Minutes, u8 /*color idx*/)>, pub kind_counts: Vec<(DayKind, u32)> }
  pub fn build_month_view(data: &MonthData, rules: &Rules, cal: &HolidayCalendar, today: NaiveDate) -> MonthView
  pub fn draw(m: &Model, f: &mut Frame, area: Rect)                         // table + summary, responsive
  pub fn draw_table(f: &mut Frame, area: Rect, t: &Theme, v: &MonthView, data: &MonthData, selected: NaiveDate, today: NaiveDate, show_comment: bool)
  pub fn draw_summary(f: &mut Frame, area: Rect, t: &Theme, v: &MonthView)
  pub fn row_index_of(v: &MonthView, date: NaiveDate) -> Option<usize>
  ```

- [ ] **Step 1: Write failing tests (bottom of `src/tui/view/month.rs`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Day, DayKind, Entry, default_tiers, HolidayCalendar, Rules};
    use crate::tui::theme::Theme;
    use crate::tui::view::testing::{contains, render};
    use chrono::{NaiveDate, NaiveTime};

    fn d(y: i32, m: u32, dd: u32) -> NaiveDate { NaiveDate::from_ymd_opt(y, m, dd).unwrap() }
    fn t(h: u32, m: u32) -> NaiveTime { NaiveTime::from_hms_opt(h, m, 0).unwrap() }

    fn fixture() -> (MonthData, Rules, HolidayCalendar) {
        let rules = Rules { daily_target: Minutes(468), tiers: default_tiers(), start_date: d(2026, 9, 1), initial_balance: Minutes::ZERO };
        let cal = HolidayCalendar::new(vec![d(2026, 9, 17)]);
        let mut days = Vec::new();
        let mut cur = d(2026, 9, 1);
        while cur <= d(2026, 9, 30) {
            let kind = if cur == d(2026, 9, 17) { DayKind::Holiday } else if cur == d(2026, 9, 2) { DayKind::Vacation } else if cur == d(2026, 9, 3) { DayKind::Flex } else { DayKind::Work };
            let entries = match cur.day() {
                1 => vec![Entry { id: 1, date: cur, start: t(8, 0), end: t(12, 0), project: "Beta".into(), comment: "half".into() }],
                14 => vec![
                    Entry { id: 2, date: cur, start: t(9, 0), end: t(12, 0), project: "Alpha".into(), comment: "morning".into() },
                    Entry { id: 3, date: cur, start: t(13, 0), end: t(16, 0), project: "Alpha".into(), comment: "afternoon".into() },
                ],
                _ => vec![],
            };
            days.push(Day { date: cur, kind, entries });
            cur = cur.succ_opt().unwrap();
        }
        let data = MonthData {
            year: 2026, month: 9, days, balance_before: Minutes::ZERO, balance_total: Minutes(-100),
            session: None, projects: vec![Project { id: 1, name: "Alpha".into(), color_index: 0, archived: false }, Project { id: 2, name: "Beta".into(), color_index: 1, archived: false }],
            vacation_used_this_year: 1,
        };
        (data, rules, cal)
    }

    #[test]
    fn builds_rows_in_spec_order() {
        let (data, rules, cal) = fixture();
        let v = build_month_view(&data, &rules, &cal, d(2026, 9, 15));
        // Tue 1: one entry row; Wed 2: vacation kind row; Thu 3: flex; Fri 4: missing; Sat/Sun 5-6: one weekend row; then week footer
        let kinds: Vec<String> = v.rows.iter().take(7).map(|r| format!("{:?}", r.kind).split(' ').next().unwrap().trim_end_matches('{').to_string()).collect();
        assert_eq!(kinds, ["Entry", "Kind", "Kind", "Missing", "Weekend", "WeekFooter", "Missing"]);
        // Mon 14 has two entry rows
        let idx = row_index_of(&v, d(2026, 9, 14)).unwrap();
        assert!(matches!(v.rows[idx].kind, RowKind::Entry { first: true, .. }));
        assert!(matches!(v.rows[idx + 1].kind, RowKind::Entry { first: false, .. }));
        // Thu 17 extra holiday
        let h = row_index_of(&v, d(2026, 9, 17)).unwrap();
        assert!(matches!(v.rows[h].kind, RowKind::Kind));
        // future day 16 is Empty
        let e = row_index_of(&v, d(2026, 9, 16)).unwrap();
        assert!(matches!(v.rows[e].kind, RowKind::Empty));
        // totals: net = 4h-18 + (6h-18) = 222 + 342 = 564; target = weekdays 1..15 with target: 1,3,4,7,8,9,10,11,14 (15=today, no entries → no target); 2 vacation, 17 future
        assert_eq!(v.net, Minutes(564));
        assert_eq!(v.target, Minutes(468 * 9));
        assert_eq!(v.project_totals[0].0, "Alpha");
        assert_eq!(v.project_totals[0].1, Minutes(342));
        assert!(v.kind_counts.iter().any(|(k, n)| *k == DayKind::Vacation && *n == 1));
    }

    #[test]
    fn week_footer_carries_running_balance() {
        let (data, rules, cal) = fixture();
        let v = build_month_view(&data, &rules, &cal, d(2026, 9, 15));
        let footers: Vec<&Row> = v.rows.iter().filter(|r| matches!(r.kind, RowKind::WeekFooter { .. })).collect();
        assert_eq!(footers.len(), 5);
        if let RowKind::WeekFooter { week, week_balance, running } = footers[0].kind {
            assert_eq!(week, 36);
            // Tue1 +222-468, Wed2 0, Thu3 -468, Fri4 -468 => -1650
            assert_eq!(week_balance, Minutes(222 - 468 - 468 - 468));
            assert_eq!(running, Minutes(222 - 468 - 468 - 468));
        } else { panic!() }
    }

    #[test]
    fn renders_table_100x30() {
        let (data, rules, cal) = fixture();
        let v = build_month_view(&data, &rules, &cal, d(2026, 9, 15));
        let t = Theme::dark();
        let rows = render(100, 40, |f| draw_table(f, f.area(), &t, &v, &data, d(2026, 9, 14), d(2026, 9, 15), true));
        assert!(contains(&rows, "Tue 01"));
        assert!(contains(&rows, "Beta"));
        assert!(contains(&rows, "08:00"));
        assert!(contains(&rows, "+03:42"));     // net of 4h
        assert!(contains(&rows, "VACATION"));
        assert!(contains(&rows, "FLEX"));
        assert!(contains(&rows, "missing"));
        assert!(contains(&rows, "weekend"));
        assert!(contains(&rows, "KW 36"));
        assert!(contains(&rows, "morning"));
        assert!(contains(&rows, "Extra holiday"));
        assert!(rows.iter().any(|r| r.contains('▶') && r.contains("Mon 14")));
    }

    #[test]
    fn hides_comment_below_90_columns() {
        let (data, rules, cal) = fixture();
        let v = build_month_view(&data, &rules, &cal, d(2026, 9, 15));
        let t = Theme::dark();
        let rows = render(85, 40, |f| draw_table(f, f.area(), &t, &v, &data, d(2026, 9, 14), d(2026, 9, 15), false));
        assert!(!contains(&rows, "morning"));
        assert!(contains(&rows, "Mon 14"));
    }

    #[test]
    fn summary_has_bars_and_totals() {
        let (data, rules, cal) = fixture();
        let v = build_month_view(&data, &rules, &cal, d(2026, 9, 15));
        let t = Theme::dark();
        let rows = render(100, 8, |f| draw_summary(f, f.area(), &t, &v));
        assert!(contains(&rows, "Alpha"));
        assert!(contains(&rows, "█"));
        assert!(contains(&rows, "60.6%"));
        assert!(contains(&rows, "target"));
        assert!(contains(&rows, "+09:24"));
        assert!(contains(&rows, "vacation 1"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `nix develop -c cargo test view::month`
Expected: compile error.

- [ ] **Step 3: Implement `build_month_view`**

```rust
use chrono::{Datelike, NaiveDate, Weekday};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout, Rect};
use tuirealm::ratatui::style::{Modifier, Style};
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Cell, Paragraph, Row as TRow, Table};

use super::{bar, block, chip, minutes_span};
use crate::core::{DayKind, DayStats, HolidayCalendar, Minutes, Project, Rules, TodayCtx, day_stats};
use crate::tui::model::Model;
use crate::tui::msg::MonthData;
use crate::tui::theme::Theme;

#[derive(Debug, Clone, PartialEq)]
pub enum RowKind {
    Entry { entry_idx: usize, first: bool },
    Missing,
    Empty,
    Kind,
    Weekend,
    WeekFooter { week: u32, week_balance: Minutes, running: Minutes },
}

#[derive(Debug, Clone)]
pub struct Row { pub date: NaiveDate, pub kind: RowKind, pub stats: Option<DayStats> }

#[derive(Debug, Clone)]
pub struct MonthView {
    pub rows: Vec<Row>,
    pub target: Minutes,
    pub net: Minutes,
    pub balance: Minutes,
    pub project_totals: Vec<(String, Minutes, u8)>,
    pub kind_counts: Vec<(DayKind, u32)>,
}

pub fn build_month_view(data: &MonthData, rules: &Rules, cal: &HolidayCalendar, today: NaiveDate) -> MonthView {
    let ctx = TodayCtx { today, clocked_in: data.session.is_some() };
    let mut rows = Vec::new();
    let (mut target, mut net) = (Minutes::ZERO, Minutes::ZERO);
    let mut running = data.balance_before;
    let mut week_balance = Minutes::ZERO;
    let mut totals: std::collections::BTreeMap<String, Minutes> = Default::default();
    let mut counts: std::collections::BTreeMap<&'static str, (DayKind, u32)> = Default::default();
    let last = data.days.last().map(|d| d.date);

    let mut i = 0;
    while i < data.days.len() {
        let day = &data.days[i];
        let s = day_stats(day, rules, cal, &ctx);
        target += s.target;
        net += s.net;
        if day.date >= rules.start_date { running += s.balance; week_balance += s.balance; }
        for e in &day.entries { *totals.entry(e.project.clone()).or_default() += e.duration(); }
        if day.kind != DayKind::Work {
            let c = counts.entry(day.kind.as_str()).or_insert((day.kind.clone(), 0));
            c.1 += 1;
        }
        if s.missing { let c = counts.entry("missing").or_insert((DayKind::Work, 0)); c.1 += 1; }

        let is_sat = day.date.weekday() == Weekday::Sat;
        let next_is_sun_empty = is_sat
            && data.days.get(i + 1).is_some_and(|n| n.date.weekday() == Weekday::Sun && n.entries.is_empty());
        if is_sat && day.entries.is_empty() && day.kind == DayKind::Work && next_is_sun_empty {
            // collapse Sat+Sun
            let sun = &data.days[i + 1];
            let ss = day_stats(sun, rules, cal, &ctx);
            rows.push(Row { date: day.date, kind: RowKind::Weekend, stats: Some(s) });
            if sun.date >= rules.start_date { running += ss.balance; week_balance += ss.balance; }
            push_footer(&mut rows, sun.date, &mut week_balance, running);
            i += 2;
            continue;
        }
        if !day.entries.is_empty() {
            for (idx, _) in day.entries.iter().enumerate() {
                rows.push(Row { date: day.date, kind: RowKind::Entry { entry_idx: idx, first: idx == 0 }, stats: Some(s.clone()) });
            }
        } else if day.kind != DayKind::Work {
            rows.push(Row { date: day.date, kind: RowKind::Kind, stats: Some(s.clone()) });
        } else if s.is_weekend {
            rows.push(Row { date: day.date, kind: RowKind::Weekend, stats: Some(s.clone()) });
        } else if s.missing {
            rows.push(Row { date: day.date, kind: RowKind::Missing, stats: Some(s.clone()) });
        } else {
            rows.push(Row { date: day.date, kind: RowKind::Empty, stats: Some(s.clone()) });
        }
        if day.date.weekday() == Weekday::Sun || Some(day.date) == last {
            push_footer(&mut rows, day.date, &mut week_balance, running);
        }
        i += 1;
    }

    // Net actually worked here excludes deductions per project; project totals use gross durations (spec: hours per project).
    let mut project_totals: Vec<(String, Minutes, u8)> = totals
        .into_iter()
        .map(|(name, m)| { let idx = data.projects.iter().find(|p| p.name == name).map(|p| p.color_index).unwrap_or(0); (name, m, idx) })
        .collect();
    project_totals.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    MonthView {
        rows,
        target,
        net,
        balance: net - target,
        project_totals,
        kind_counts: counts.into_values().collect(),
    }
}

fn push_footer(rows: &mut Vec<Row>, date: NaiveDate, week_balance: &mut Minutes, running: Minutes) {
    let week = date.iso_week().week();
    rows.push(Row { date, kind: RowKind::WeekFooter { week, week_balance: *week_balance, running }, stats: None });
    *week_balance = Minutes::ZERO;
}

pub fn row_index_of(v: &MonthView, date: NaiveDate) -> Option<usize> {
    v.rows.iter().position(|r| r.date == date && !matches!(r.kind, RowKind::WeekFooter { .. }))
        .or_else(|| v.rows.iter().position(|r| matches!(r.kind, RowKind::Weekend) && (r.date == date || r.date.succ_opt() == Some(date))))
}
```

Note on the `builds_rows_in_spec_order` test: with the fixture, day 15 (today, no entries) is `Empty`, and `week 36` contains Tue 1..Sun 6. Adjust the `kinds` extraction if `format!("{:?}")` differs; the intent is the sequence `Entry, Kind, Kind, Missing, Weekend, WeekFooter, Missing`. The `net` test value: day 1 net = 240−18 = 222; day 14 gross = 360 → deduction 18 → 342; sum 564. Project totals use **gross** entry durations: Alpha 360, Beta 240 → the summary test expects `60.6%` = 342/564? No — use **net share by gross**: Alpha 360/600 = 60.0%. Fix the test expectation to `"60.0%"` and `project_totals[0].1 == Minutes(360)`.

- [ ] **Step 4: Implement `draw_table`, `draw_summary`, `draw`**

```rust
const DAY_W: u16 = 9;

pub fn draw(m: &Model, f: &mut Frame, area: Rect) {
    let Some(data) = &m.month else {
        f.render_widget(Paragraph::new("loading…").style(Style::default().fg(m.theme.muted)), area);
        return;
    };
    let v = build_month_view(data, &m.rules, &m.cal, m.today);
    let stacked = area.width < 100;
    let summary_h = (v.project_totals.len() as u16 + 4).max(5);
    let [table_a, summary_a] = if stacked {
        Layout::vertical([Constraint::Min(8), Constraint::Length(summary_h.min(area.height / 3))]).areas(area)
    } else {
        Layout::vertical([Constraint::Min(8), Constraint::Length(summary_h)]).areas(area)
    };
    draw_table(f, table_a, &m.theme, &v, data, m.selected, m.today, area.width >= 90);
    draw_summary(f, summary_a, &m.theme, &v);
}

fn day_label(date: NaiveDate) -> String {
    format!("{} {:02}", date.format("%a"), date.day())
}

pub fn draw_table(f: &mut Frame, area: Rect, t: &Theme, v: &MonthView, data: &MonthData, selected: NaiveDate, today: NaiveDate, show_comment: bool) {
    let inner_w = area.width.saturating_sub(2);
    let mut widths: Vec<Constraint> = vec![
        Constraint::Length(DAY_W), Constraint::Length(16), Constraint::Length(7), Constraint::Length(7), Constraint::Length(8), Constraint::Length(8),
    ];
    if show_comment { widths.push(Constraint::Min(10)); }
    let fixed: u16 = DAY_W + 16 + 7 + 7 + 8 + 8 + 6;
    let comment_w = inner_w.saturating_sub(fixed) as usize;
    let span_w = inner_w.saturating_sub(DAY_W + 1) as usize; // width of a banner spanning all columns after DAY

    let header = TRow::new(["DAY", "PROJECT", "START", "END", "GROSS", "NET", "COMMENT"].iter().take(widths.len())
        .map(|h| Cell::from(Span::styled(*h, Style::default().fg(t.muted).add_modifier(Modifier::BOLD)))));

    let sel_idx = row_index_of(v, selected);
    let rows: Vec<TRow> = v.rows.iter().enumerate().map(|(i, r)| {
        let is_sel = Some(i) == sel_idx || (sel_idx.is_some() && matches!(r.kind, RowKind::Entry { first: false, .. }) && v.rows[..i].iter().rposition(|x| matches!(x.kind, RowKind::Entry { first: true, .. })).and_then(|p| sel_idx.map(|s| s == p)).unwrap_or(false));
        let is_today = r.date == today;
        let mark = if is_sel { "▶" } else { " " };
        let day_style = if is_today { Style::default().fg(t.accent).add_modifier(Modifier::BOLD) } else if matches!(r.kind, RowKind::Missing) { Style::default().fg(t.negative) } else { Style::default().fg(t.text) };
        let day_cell = |show: bool| Cell::from(Line::from(vec![Span::styled(mark, Style::default().fg(t.accent)), Span::styled(if show { day_label(r.date) } else { String::new() }, day_style)]));
        let empty = || Cell::from("");
        let banner = |text: Line<'static>| -> Vec<Cell> {
            let mut cells = vec![day_cell(true), Cell::from(text)];
            cells.resize_with(widths.len(), empty);
            cells
        };
        let cells: Vec<Cell> = match &r.kind {
            RowKind::Entry { entry_idx, first } => {
                let e = &data.days.iter().find(|d| d.date == r.date).unwrap().entries[*entry_idx];
                let color = data.projects.iter().find(|p| p.name == e.project).map(|p| t.project_color(p.color_index)).unwrap_or(t.text);
                let s = r.stats.as_ref().unwrap();
                let mut c = vec![
                    day_cell(*first),
                    Cell::from(Span::styled(truncate(&e.project, 15), Style::default().fg(color))),
                    Cell::from(e.start.format("%H:%M").to_string()),
                    Cell::from(e.end.format("%H:%M").to_string()),
                    Cell::from(Span::styled(e.duration().to_string(), Style::default().fg(t.text))),
                    Cell::from(if *first { minutes_span(s.net, t) } else { Span::raw("") }),
                ];
                if show_comment { c.push(Cell::from(Span::styled(truncate(&e.comment, comment_w), Style::default().fg(t.muted)))); }
                c
            }
            RowKind::Missing => banner(Line::from(chip("missing", t.negative))),
            RowKind::Empty => banner(Line::from(Span::styled("·", Style::default().fg(t.muted)))),
            RowKind::Weekend => banner(Line::from(Span::styled("weekend", Style::default().fg(t.muted)))),
            RowKind::Kind => {
                let s = r.stats.as_ref().unwrap();
                let label = s.kind.display_name().to_uppercase();
                let extra = match &s.kind { DayKind::Holiday => s.holiday_name.clone().unwrap_or_default(), DayKind::Flex => format!("{}", -s.target), _ => String::new() };
                banner(Line::from(vec![chip(&label, t.kind_color(&s.kind)), Span::raw("  "), Span::styled(extra, Style::default().fg(t.muted))]))
            }
            RowKind::WeekFooter { week, week_balance, running } => {
                let text = Line::from(vec![
                    Span::styled(format!("KW {week:02}  "), Style::default().fg(t.muted)), minutes_span(*week_balance, t),
                    Span::styled("  →  ", Style::default().fg(t.muted)), minutes_span(*running, t),
                ]);
                let pad = span_w.saturating_sub(text.width());
                let mut cells = vec![empty(), Cell::from(Line::from(std::iter::once(Span::raw(" ".repeat(pad))).chain(text.spans).collect::<Vec<_>>()))];
                cells.resize_with(widths.len(), empty);
                cells
            }
        };
        let mut row = TRow::new(cells);
        if is_sel { row = row.style(Style::default().bg(t.bg_selected)); }
        if matches!(r.kind, RowKind::WeekFooter { .. }) { row = row.bottom_margin(0); }
        row
    }).collect();

    // Keep the selected row visible: offset so that selected is within the viewport.
    let viewport = area.height.saturating_sub(3) as usize; // borders + header
    let offset = sel_idx.map(|s| s.saturating_sub(viewport.saturating_sub(1) / 2)).unwrap_or(0).min(v.rows.len().saturating_sub(viewport));
    let visible: Vec<TRow> = rows.into_iter().skip(offset).collect();

    let table = Table::new(visible, widths).header(header).column_spacing(1).block(block(t, None));
    f.render_widget(table, area);
}

pub fn draw_summary(f: &mut Frame, area: Rect, t: &Theme, v: &MonthView) {
    let total: i32 = v.project_totals.iter().map(|p| p.1.0).sum::<i32>().max(1);
    let bar_w = area.width.saturating_sub(2 + 18 + 2 + 8 + 2 + 8 + 2).max(10);
    let mut lines: Vec<Line> = v.project_totals.iter().map(|(name, m, idx)| {
        let frac = m.0 as f64 / total as f64;
        let mut l = vec![Span::styled(format!("{:<18}", truncate(name, 18)), Style::default().fg(t.project_color(*idx)))];
        l.extend(bar(frac, bar_w, t.project_color(*idx), t).spans);
        l.push(Span::raw(format!("  {:>8}  {:>5.1}%", m.hhmm(), frac * 100.0)));
        Line::from(l)
    }).collect();
    lines.push(Line::from(vec![
        Span::styled("target ", Style::default().fg(t.muted)), Span::raw(format!("-{}", v.target.hhmm())),
        Span::styled("   net ", Style::default().fg(t.muted)), Span::raw(format!("+{}", v.net.hhmm())),
        Span::styled("   month ", Style::default().fg(t.muted)), minutes_span(v.balance, t),
    ]));
    let counts: Vec<String> = v.kind_counts.iter().map(|(k, n)| format!("{} {n}", k.display_name().to_lowercase())).collect();
    if !counts.is_empty() {
        lines.push(Line::from(Span::styled(counts.join("   "), Style::default().fg(t.muted))));
    }
    f.render_widget(Paragraph::new(lines).block(block(t, Some("Month"))), area);
}

fn truncate(s: &str, w: usize) -> String {
    if s.chars().count() <= w { s.to_string() } else { let mut o: String = s.chars().take(w.saturating_sub(1)).collect(); o.push('…'); o }
}
```

`kind_counts` for missing days: the label should read `missing N`; give `DayKind::Work` in `counts` the key `"missing"` and render it as `missing` — implement by storing `(DayKind, u32)` plus mapping `Work → "missing"` in `draw_summary`.

- [ ] **Step 5: Run tests; fix expectations to actual values only if arithmetic in the test was wrong (recheck by hand first)**

Run: `nix develop -c cargo test view::month`
Expected: 5 passed.

- [ ] **Step 6: Manual check, then commit**

Run `TK_HOME=/tmp/tk-smoke nix develop -c cargo run` after `tk --home /tmp/tk-smoke add today 0900-1200 -p Alpha -m hello`: today's row shows Alpha, `09:00`, `12:00`, `+03:00`, `+02:42`, "hello"; the summary shows one bar; `↓` moves the ▶ marker; `[` shows the previous month with missing chips only after `start_date`.

```bash
./scripts/check.sh
git add src/tui
git commit -m "feat(tui): month view with entries, kinds, week footers, summary"
```

---

### Task 14: Month view actions (clock in/out, day kinds, confirm)

**Files:**
- Modify: `src/tui/model.rs` (`update`), `src/tui/components/mod.rs` (add `confirm.rs`, `help.rs`), create `src/tui/components/confirm.rs`, `src/tui/components/help.rs`
- Modify: `src/tui/mod.rs` (mount `Id::Confirm`, `Id::Help`)

**Interfaces:**
- Produces: `ConfirmDialog` component (`y`/`Enter` → `ConfirmYes`, `n`/`Esc` → `ConfirmNo`), `HelpOverlay` component (any key → `ToggleHelp`), and `Model::update` arms for `ClockIn`, `ClockOut`, `SetKind`, `AskConfirm`, `ConfirmYes`, `ConfirmNo`, `ToggleHelp` with focus handling `Model::focus(&mut self, id: Id)`.

- [ ] **Step 1: Write failing unit tests for `update` (bottom of `src/tui/model.rs`)**

These tests build a `Model` without a terminal. Add a test constructor:

```rust
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::tui::msg::StoreCmd;
    use std::sync::mpsc::{Receiver, channel};

    pub fn model(today: NaiveDate) -> (Model, Receiver<StoreCmd>) {
        let (tx, rx) = channel();
        let app: Application<Id, Msg, UserEvent> = Application::init(tuirealm::listener::EventListenerCfg::default());
        let m = Model {
            app, terminal: None, theme: Theme::dark(),
            rules: Rules { daily_target: crate::core::Minutes(468), tiers: crate::core::default_tiers(), start_date: today, initial_balance: crate::core::Minutes::ZERO },
            cal: HolidayCalendar::default(), vacation_allowance: 30, screen: Screen::Month, selected: today, today,
            now: NaiveTime::from_hms_opt(10, 0, 0).unwrap(), month: None, day: None, stats: None, day_cursor: 0,
            confirm: None, help: false, status: None, quit: false, redraw: false, worker: Worker { tx }, size: (100, 30),
        };
        (m, rx)
    }

    pub fn month_data(today: NaiveDate, days: Vec<crate::core::Day>, session: Option<crate::store::Session>) -> MonthData {
        MonthData { year: today.year(), month: today.month(), days, balance_before: Default::default(), balance_total: Default::default(), session, projects: vec![], vacation_used_this_year: 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use crate::core::{Day, DayKind};
    use crate::tui::msg::StoreCmd;
    use chrono::NaiveDate;

    fn d(y: i32, m: u32, dd: u32) -> NaiveDate { NaiveDate::from_ymd_opt(y, m, dd).unwrap() }

    #[test]
    fn set_kind_asks_confirm_then_sends_cmd() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        m.month = Some(month_data(today, vec![Day { date: today, kind: DayKind::Work, entries: vec![] }], None));
        m.update(Msg::SetKind(DayKind::Vacation));
        assert_eq!(m.confirm, Some(Confirm::SetKind(today, DayKind::Vacation)));
        m.update(Msg::ConfirmYes);
        assert_eq!(m.confirm, None);
        assert!(matches!(rx.try_recv().unwrap(), StoreCmd::SetKind(dt, DayKind::Vacation) if dt == today));
    }

    #[test]
    fn set_kind_on_day_with_entries_is_refused() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        let e = crate::core::Entry { id: 1, date: today, start: chrono::NaiveTime::from_hms_opt(8, 0, 0).unwrap(), end: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(), project: "A".into(), comment: "".into() };
        m.month = Some(month_data(today, vec![Day { date: today, kind: DayKind::Work, entries: vec![e] }], None));
        m.update(Msg::SetKind(DayKind::Sick));
        assert!(m.confirm.is_none());
        assert!(m.status.as_ref().unwrap().1);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn clock_in_out_flow() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        m.month = Some(month_data(today, vec![], None));
        m.update(Msg::ClockIn);
        assert!(matches!(rx.try_recv().unwrap(), StoreCmd::ClockIn(dt, _) if dt == today));
        m.update(Msg::ClockOut);
        assert!(m.status.as_ref().unwrap().1); // not clocked in → error status
        let sess = crate::store::Session { date: today, start: chrono::NaiveTime::from_hms_opt(8, 0, 0).unwrap(), project: None };
        m.month = Some(month_data(today, vec![], Some(sess)));
        m.update(Msg::ClockIn);
        assert_eq!(m.confirm, Some(Confirm::ClockInReplace));
        m.update(Msg::ConfirmNo);
        assert!(m.confirm.is_none());
        m.update(Msg::ClockOut);
        assert!(matches!(rx.try_recv().unwrap(), StoreCmd::ClockOut { .. }));
    }

    #[test]
    fn clock_in_on_weekend_is_refused() {
        let sat = d(2026, 9, 19);
        let (mut m, rx) = model(sat);
        m.month = Some(month_data(sat, vec![], None));
        m.update(Msg::ClockIn);
        assert!(rx.try_recv().is_err());
        assert!(m.status.as_ref().unwrap().1);
    }

    #[test]
    fn navigation_and_month_change() {
        let today = d(2026, 1, 31);
        let (mut m, rx) = model(today);
        m.update(Msg::SelectMonth(1));
        assert_eq!(m.selected, d(2026, 2, 28));
        assert!(matches!(rx.try_recv().unwrap(), StoreCmd::LoadMonth { year: 2026, month: 2 }));
        m.update(Msg::SelectMonth(-2));
        assert_eq!(m.selected, d(2025, 12, 28));
        m.update(Msg::GoToday);
        assert_eq!(m.selected, today);
        m.update(Msg::SelectDay(1));
        assert_eq!(m.selected, d(2026, 2, 1));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `nix develop -c cargo test tui::model`
Expected: `set_kind_*` and `clock_*` tests fail (arms are `_ => {}`); navigation passes.

- [ ] **Step 3: Implement the arms in `Model::update`**

Replace the `_ => {}` arm with:

```rust
            Msg::ClockIn => {
                if !crate::core::is_working_day(self.today) { self.set_status("Cannot clock in on a weekend", true); return; }
                if self.month.as_ref().is_some_and(|m| m.session.is_some()) { self.open_confirm(Confirm::ClockInReplace); return; }
                self.worker.send(StoreCmd::ClockIn(self.today, self.now.with_second(0).unwrap()));
            }
            Msg::ClockOut => {
                if !self.month.as_ref().is_some_and(|m| m.session.is_some()) { self.set_status("Not clocked in", true); return; }
                self.worker.send(StoreCmd::ClockOut { project: None, comment: String::new() });
            }
            Msg::SetKind(kind) => {
                let has_entries = self.month.as_ref().and_then(|m| m.days.iter().find(|d| d.date == self.selected)).is_some_and(|d| !d.entries.is_empty());
                if has_entries && kind != DayKind::Work { self.set_status("Day has entries; delete them first", true); return; }
                if !crate::core::is_working_day(self.selected) && kind != DayKind::Work { self.set_status("Weekends need no day type", true); return; }
                self.open_confirm(Confirm::SetKind(self.selected, kind));
            }
            Msg::AskConfirm(c) => self.open_confirm(c),
            Msg::ConfirmNo => self.close_overlay(),
            Msg::ConfirmYes => {
                let Some(c) = self.confirm.take() else { return };
                self.close_overlay();
                match c {
                    Confirm::DeleteEntry(id) => self.worker.send(StoreCmd::DeleteEntry(id)),
                    Confirm::SetKind(d, k) => self.worker.send(StoreCmd::SetKind(d, k)),
                    Confirm::ClockInReplace => {
                        // clear then clock in: worker handles sequentially
                        self.worker.send(StoreCmd::ClockOut { project: None, comment: String::new() });
                        self.worker.send(StoreCmd::ClockIn(self.today, self.now.with_second(0).unwrap()));
                    }
                }
            }
            Msg::ToggleHelp => { self.help = !self.help; if self.help { self.focus(Id::Help) } else { self.focus_screen() } }
            _ => {}
```

`ClockInReplace` semantics: replacing means the running session is discarded, not recorded. Add `StoreCmd::ClearSession` to `msg.rs` and `worker.rs` (`ctx.store.clear_session()?; StoreReply::Changed(String::new())`) and use it instead of `ClockOut` above.

Add helpers to `impl Model`:

```rust
    pub fn focus(&mut self, id: Id) { let _ = self.app.active(&id); }
    pub fn focus_screen(&mut self) {
        let id = match self.screen { Screen::Month => Id::Month, Screen::Day => Id::Day, Screen::Stats => Id::Stats };
        self.focus(id);
    }
    fn open_confirm(&mut self, c: Confirm) { self.confirm = Some(c); self.focus(Id::Confirm); }
    fn close_overlay(&mut self) { self.confirm = None; self.help = false; self.focus_screen(); }
```

Also import `chrono::Timelike` for `with_second`. In the test constructor, `app.active` on an unmounted id returns `Err`, which `focus` ignores; that is intended.

- [ ] **Step 4: Components `confirm.rs` and `help.rs`**

```rust
// confirm.rs
use tuirealm::component::AppComponent;
use tuirealm::event::{Event, Key, KeyEvent};
use super::KeyOnly;
use crate::tui::msg::{Msg, UserEvent};

#[derive(Default, tuirealm::Component)]
pub struct ConfirmDialog { component: KeyOnly }

impl AppComponent<Msg, UserEvent> for ConfirmDialog {
    fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg> {
        let KeyEvent { code, .. } = ev.as_keyboard()?;
        match code {
            Key::Char('y') | Key::Enter => Some(Msg::ConfirmYes),
            Key::Char('n') | Key::Esc | Key::Char('q') => Some(Msg::ConfirmNo),
            _ => None,
        }
    }
}

// help.rs
#[derive(Default, tuirealm::Component)]
pub struct HelpOverlay { component: KeyOnly }

impl AppComponent<Msg, UserEvent> for HelpOverlay {
    fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg> {
        ev.as_keyboard().map(|_| Msg::ToggleHelp)
    }
}
```

Mount both in `tui::run` after `Id::Month`: `app.mount(Id::Confirm, Box::new(ConfirmDialog::default()), vec![])?; app.mount(Id::Help, Box::new(HelpOverlay::default()), vec![])?;`.

- [ ] **Step 5: Run tests, manual check, commit**

Run: `nix develop -c cargo test tui::model` → 5 passed.
Manual: `i` shows "Clocked in at HH:MM" and the title bar timer counts; `o` records; `v` on a future weekday asks "Set … to vacation?" and `y` shows the VACATION chip; `?` opens help and any key closes it.

```bash
./scripts/check.sh
git add src/tui
git commit -m "feat(tui): clock in/out, day kinds with confirm, help overlay"
```

---

### Task 15: Day editor screen

**Files:**
- Create: `src/tui/view/day.rs` (replace stub), `src/tui/components/day.rs` (replace stub)
- Modify: `src/tui/model.rs` (`OpenDay`, `DaySelect`, `DayKindNext/Prev`, `DayDelete`, `Back` from Day), `src/tui/mod.rs` (mount `Id::Day`)

**Interfaces:**
- Produces:
  ```rust
  // view/day.rs
  pub fn draw(m: &Model, f: &mut Frame, area: Rect)
  pub fn draw_day(f: &mut Frame, area: Rect, t: &Theme, data: &DayData, stats: &DayStats, cursor: usize, kinds: &[DayKind], kind_idx: usize)
  pub const KIND_CYCLE: [&str; 6] = ["work", "vacation", "flex", "holiday", "sick", "absence"];
  // components/day.rs
  pub struct DayScreen  // keys: ↑/k DaySelect(-1), ↓/j DaySelect(1), a DayAdd, e/Enter DayEdit, d DayDelete, ←/h DayKindPrev, →/l DayKindNext, ? ToggleHelp, Esc/q Back
  ```

- [ ] **Step 1: Write failing tests**

Snapshot test in `view/day.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Day, DayKind, Entry, HolidayCalendar, Minutes, Rules, TodayCtx, day_stats, default_tiers};
    use crate::tui::msg::DayData;
    use crate::tui::theme::Theme;
    use crate::tui::view::testing::{contains, render};
    use chrono::{NaiveDate, NaiveTime};

    #[test]
    fn renders_entries_kind_selector_and_footer() {
        let date = NaiveDate::from_ymd_opt(2026, 9, 14).unwrap();
        let t = |h, m| NaiveTime::from_hms_opt(h, m, 0).unwrap();
        let day = Day { date, kind: DayKind::Work, entries: vec![
            Entry { id: 1, date, start: t(9, 0), end: t(12, 0), project: "Alpha".into(), comment: "morning".into() },
            Entry { id: 2, date, start: t(13, 0), end: t(16, 0), project: "Beta".into(), comment: "".into() },
        ]};
        let rules = Rules { daily_target: Minutes(468), tiers: default_tiers(), start_date: date, initial_balance: Minutes::ZERO };
        let stats = day_stats(&day, &rules, &HolidayCalendar::default(), &TodayCtx { today: date.succ_opt().unwrap(), clocked_in: false });
        let data = DayData { day, projects: vec![] };
        let rows = render(100, 24, |f| draw_day(f, f.area(), &Theme::dark(), &data, &stats, 1, &KIND_CYCLE, 0));
        assert!(contains(&rows, "Monday, 14 September 2026"));
        assert!(contains(&rows, "[work]"));
        assert!(contains(&rows, "vacation"));
        assert!(contains(&rows, "Alpha"));
        assert!(rows.iter().any(|r| r.contains('▶') && r.contains("Beta")));
        assert!(contains(&rows, "gross +06:00"));
        assert!(contains(&rows, "break -00:18"));
        assert!(contains(&rows, "net +05:42"));
    }
}
```

Model tests (append to `src/tui/model.rs` tests):

```rust
    #[test]
    fn open_day_loads_and_focuses() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        m.update(Msg::OpenDay);
        assert_eq!(m.screen, Screen::Day);
        assert!(matches!(rx.try_recv().unwrap(), StoreCmd::LoadDay(dt) if dt == today));
        m.update(Msg::Back);
        assert_eq!(m.screen, Screen::Month);
    }

    #[test]
    fn day_delete_asks_confirm_for_selected_entry() {
        let today = d(2026, 9, 15);
        let (mut m, _rx) = model(today);
        let t = |h| chrono::NaiveTime::from_hms_opt(h, 0, 0).unwrap();
        let e = |id, s, e| crate::core::Entry { id, date: today, start: t(s), end: t(e), project: "A".into(), comment: "".into() };
        m.screen = Screen::Day;
        m.day = Some(crate::tui::msg::DayData { day: Day { date: today, kind: DayKind::Work, entries: vec![e(1, 8, 9), e(2, 10, 11)] }, projects: vec![] });
        m.update(Msg::DaySelect(1));
        m.update(Msg::DayDelete);
        assert_eq!(m.confirm, Some(Confirm::DeleteEntry(2)));
    }

    #[test]
    fn day_kind_cycle_sends_set_kind() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        m.screen = Screen::Day;
        m.day = Some(crate::tui::msg::DayData { day: Day { date: today, kind: DayKind::Work, entries: vec![] }, projects: vec![] });
        m.update(Msg::DayKindNext);
        assert!(matches!(rx.try_recv().unwrap(), StoreCmd::SetKind(_, DayKind::Vacation)));
        m.day.as_mut().unwrap().day.kind = DayKind::Vacation;
        m.update(Msg::DayKindPrev);
        assert!(matches!(rx.try_recv().unwrap(), StoreCmd::SetKind(_, DayKind::Work)));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `nix develop -c cargo test tui`
Expected: new tests fail to compile / fail.

- [ ] **Step 3: Implement `view/day.rs`**

```rust
use chrono::NaiveDate;
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout, Rect};
use tuirealm::ratatui::style::{Modifier, Style};
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Cell, Paragraph, Row, Table};

use super::{block, chip, minutes_span};
use crate::core::{DayKind, DayStats, TodayCtx, day_stats};
use crate::tui::model::Model;
use crate::tui::msg::DayData;
use crate::tui::theme::Theme;

pub const KIND_CYCLE: [&str; 6] = ["work", "vacation", "flex", "holiday", "sick", "absence"];

pub fn kind_index(k: &DayKind) -> usize { KIND_CYCLE.iter().position(|s| *s == k.as_str()).unwrap_or(0) }

pub fn draw(m: &Model, f: &mut Frame, area: Rect) {
    let Some(data) = &m.day else {
        f.render_widget(Paragraph::new("loading…").style(Style::default().fg(m.theme.muted)), area);
        return;
    };
    let stats = day_stats(&data.day, &m.rules, &m.cal, &TodayCtx { today: m.today, clocked_in: m.month.as_ref().is_some_and(|x| x.session.is_some()) });
    draw_day(f, area, &m.theme, data, &stats, m.day_cursor, &KIND_CYCLE, kind_index(&data.day.kind));
}

fn long_date(d: NaiveDate) -> String { d.format("%A, %-d %B %Y").to_string() }

pub fn draw_day(f: &mut Frame, area: Rect, t: &Theme, data: &DayData, stats: &DayStats, cursor: usize, kinds: &[&str], kind_idx: usize) {
    let [head, kind_a, table_a, foot] = Layout::vertical([Constraint::Length(2), Constraint::Length(3), Constraint::Min(4), Constraint::Length(3)]).areas(area);

    let mut title = vec![Span::styled(long_date(data.day.date), Style::default().fg(t.accent).add_modifier(Modifier::BOLD))];
    if let Some(h) = &stats.holiday_name { title.push(Span::styled(format!("   {h}"), Style::default().fg(t.chip_holiday))); }
    f.render_widget(Paragraph::new(Line::from(title)), head);

    let mut ks: Vec<Span> = vec![Span::styled("← → ", Style::default().fg(t.muted))];
    for (i, k) in kinds.iter().enumerate() {
        let sel = i == kind_idx;
        let text = if sel { format!("[{k}]") } else { format!(" {k} ") };
        let style = if sel { Style::default().fg(t.kind_color(&DayKind::parse(k, None).unwrap())).add_modifier(Modifier::BOLD) } else { Style::default().fg(t.muted) };
        ks.push(Span::styled(text, style));
        ks.push(Span::raw(" "));
    }
    f.render_widget(Paragraph::new(Line::from(ks)).block(block(t, Some("Day type"))), kind_a);

    let header = Row::new(["", "START", "END", "GROSS", "PROJECT", "COMMENT"].map(|h| Cell::from(Span::styled(h, Style::default().fg(t.muted).add_modifier(Modifier::BOLD)))));
    let rows: Vec<Row> = data.day.entries.iter().enumerate().map(|(i, e)| {
        let color = data.projects.iter().find(|p| p.name == e.project).map(|p| t.project_color(p.color_index)).unwrap_or(t.text);
        let mut r = Row::new(vec![
            Cell::from(if i == cursor { "▶" } else { " " }),
            Cell::from(e.start.format("%H:%M").to_string()),
            Cell::from(e.end.format("%H:%M").to_string()),
            Cell::from(e.duration().to_string()),
            Cell::from(Span::styled(e.project.clone(), Style::default().fg(color))),
            Cell::from(Span::styled(e.comment.clone(), Style::default().fg(t.muted))),
        ]);
        if i == cursor { r = r.style(Style::default().bg(t.bg_selected)); }
        r
    }).collect();
    let empty_hint = if data.day.entries.is_empty() { Some("no entries — press a to add") } else { None };
    let table = Table::new(rows, [Constraint::Length(1), Constraint::Length(6), Constraint::Length(6), Constraint::Length(7), Constraint::Length(18), Constraint::Min(10)])
        .header(header).column_spacing(1).block(block(t, Some("Entries")));
    f.render_widget(table, table_a);
    if let Some(h) = empty_hint {
        let inner = Rect { x: table_a.x + 2, y: table_a.y + 2, width: table_a.width.saturating_sub(4), height: 1 };
        f.render_widget(Paragraph::new(Span::styled(h, Style::default().fg(t.muted))), inner);
    }

    let mut foot_line = vec![
        Span::styled("gross ", Style::default().fg(t.muted)), Span::raw(stats.gross.to_string()),
        Span::styled("   break ", Style::default().fg(t.muted)), Span::raw(format!("-{}", stats.deduction.hhmm())),
        Span::styled("   net ", Style::default().fg(t.muted)), minutes_span(stats.net, t),
        Span::styled("   target ", Style::default().fg(t.muted)), Span::raw(format!("-{}", stats.target.hhmm())),
        Span::styled("   day ", Style::default().fg(t.muted)), minutes_span(stats.balance, t),
    ];
    if data.day.kind != DayKind::Work { foot_line.insert(0, chip(&data.day.kind.display_name().to_uppercase(), t.kind_color(&data.day.kind))); foot_line.insert(1, Span::raw("  ")); }
    f.render_widget(Paragraph::new(Line::from(foot_line)).block(block(t, None)), foot);
}
```

- [ ] **Step 4: Implement `components/day.rs` and model arms**

```rust
use tuirealm::component::AppComponent;
use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers};
use super::KeyOnly;
use crate::tui::msg::{Msg, UserEvent};

#[derive(Default, tuirealm::Component)]
pub struct DayScreen { component: KeyOnly }

impl AppComponent<Msg, UserEvent> for DayScreen {
    fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg> {
        let KeyEvent { code, modifiers } = ev.as_keyboard()?;
        if *modifiers == KeyModifiers::CONTROL && *code == Key::Char('c') { return Some(Msg::Quit); }
        Some(match code {
            Key::Up | Key::Char('k') => Msg::DaySelect(-1),
            Key::Down | Key::Char('j') => Msg::DaySelect(1),
            Key::Char('a') => Msg::DayAdd,
            Key::Char('e') | Key::Enter => Msg::DayEdit,
            Key::Char('d') | Key::Delete => Msg::DayDelete,
            Key::Left | Key::Char('h') => Msg::DayKindPrev,
            Key::Right | Key::Char('l') => Msg::DayKindNext,
            Key::Char('?') => Msg::ToggleHelp,
            Key::Esc | Key::Char('q') => Msg::Back,
            _ => return None,
        })
    }
}
```

Model arms (add to `update`):

```rust
            Msg::OpenDay => { self.screen = Screen::Day; self.day = None; self.day_cursor = 0; self.worker.send(StoreCmd::LoadDay(self.selected)); self.focus(Id::Day); }
            Msg::DaySelect(n) => {
                let len = self.day.as_ref().map(|d| d.day.entries.len()).unwrap_or(0);
                if len > 0 { self.day_cursor = (self.day_cursor as i32 + n).rem_euclid(len as i32) as usize; }
            }
            Msg::DayDelete => {
                if let Some(e) = self.day.as_ref().and_then(|d| d.day.entries.get(self.day_cursor)) { self.open_confirm(Confirm::DeleteEntry(e.id)); }
            }
            Msg::DayKindNext | Msg::DayKindPrev => {
                let Some(d) = &self.day else { return };
                if !d.day.entries.is_empty() { self.set_status("Day has entries; delete them first", true); return; }
                if !crate::core::is_working_day(d.day.date) { self.set_status("Weekends need no day type", true); return; }
                let cur = super::view::day::kind_index(&d.day.kind) as i32;
                let next = (cur + if matches!(msg, Msg::DayKindNext) { 1 } else { -1 }).rem_euclid(6) as usize;
                let kind = DayKind::parse(super::view::day::KIND_CYCLE[next], Some("absence")).unwrap();
                self.worker.send(StoreCmd::SetKind(d.day.date, kind));
            }
```

Note: `msg` is moved by the `match`; bind with `match &msg` or capture `let is_next = matches!(msg, Msg::DayKindNext);` before the match. Also make `Back` from `Screen::Day` reload the month (it already sends `LoadMonth` via `Changed`, but a plain `Esc` should call `self.load_month()` too). `on_store(StoreReply::Day(d))` must clamp `day_cursor` to `d.day.entries.len().saturating_sub(1)`.

Mount `Id::Day` in `tui::run`: `app.mount(Id::Day, Box::new(components::day::DayScreen::default()), vec![])?;`.

- [ ] **Step 5: Run tests, manual check, commit**

Run: `nix develop -c cargo test tui` → all pass.
Manual: `Enter` on a day opens the editor with its entries; `←`/`→` cycles the type on an empty day; `d` asks to delete; `Esc` returns with the month refreshed.

```bash
./scripts/check.sh
git add src/tui
git commit -m "feat(tui): day editor screen with kind selector and entry table"
```

---

### Task 16: Entry form overlay with project picker

**Files:**
- Create: `src/tui/components/form.rs` (replace stub)
- Modify: `src/tui/model.rs` (`DayAdd`, `DayEdit`, `FormSubmit`, `FormCancel`, `Model::form: Option<FormState>`), `src/tui/view/day.rs` (draw the overlay when `m.form.is_some()`), `src/tui/mod.rs` (mount `Id::Form`)

**Interfaces:**
- Produces:
  ```rust
  // components/form.rs
  pub struct EntryForm { fields: [tui_realm_stdlib::Input; 4], focus: usize, id: Option<i64>, projects: Vec<String>, picker_open: bool, picker_idx: usize, filter: String }
  impl EntryForm { pub fn new(initial: FormData, projects: Vec<String>) -> Self; pub fn data(&self) -> FormData }
  // keys: Tab/↓ next field, BackTab/↑ previous, Enter on last field or Ctrl+S → FormSubmit(data), Esc → FormCancel;
  // in the Project field: typing filters, ↓/↑ moves in the picker, Enter accepts the highlighted project (or the typed text as a new project).
  // view: draws itself (it is the only component that draws) — a centered 60×12 box with 4 labelled inputs and a live footer line supplied via Attribute::Text ("gross +03:00 · break -00:18 · net +02:42" or an error in red).
  // model.rs
  pub struct FormState { pub data: FormData, pub error: Option<String>, pub preview: Option<(Minutes, Minutes, Minutes)> }
  pub fn validate_form(d: &FormData, existing: &[Entry], rules: &Rules) -> Result<(NaiveTime, NaiveTime, Minutes, Minutes, Minutes), String>  // start, end, gross, deduction, net(day)
  ```

- [ ] **Step 1: Write failing tests for `validate_form` (in `model.rs` tests)**

```rust
    #[test]
    fn validate_form_cases() {
        let today = d(2026, 9, 15);
        let (m, _rx) = model(today);
        let t = |h, mi| chrono::NaiveTime::from_hms_opt(h, mi, 0).unwrap();
        let existing = vec![crate::core::Entry { id: 1, date: today, start: t(8, 0), end: t(12, 0), project: "A".into(), comment: "".into() }];
        let fd = |s: &str, e: &str, p: &str| FormData { id: None, start: s.into(), end: e.into(), project: p.into(), comment: "".into() };
        let ok = validate_form(&fd("1300", "16:00", "Alpha"), &existing, &m.rules).unwrap();
        assert_eq!(ok.0, t(13, 0));
        assert_eq!(ok.2, Minutes(180));        // gross of this entry
        assert_eq!(ok.3, Minutes(48));         // day deduction with 7h total
        assert_eq!(ok.4, Minutes(420 - 48));   // day net
        assert!(validate_form(&fd("abc", "1600", "A"), &existing, &m.rules).unwrap_err().contains("start"));
        assert!(validate_form(&fd("1300", "", "A"), &existing, &m.rules).unwrap_err().contains("end"));
        assert!(validate_form(&fd("1300", "1600", " "), &existing, &m.rules).unwrap_err().contains("project"));
        assert!(validate_form(&fd("1100", "1300", "A"), &existing, &m.rules).unwrap_err().contains("overlap"));
        // editing entry 1 itself may overlap its old slot
        let mut edit = fd("0900", "1200", "A"); edit.id = Some(1);
        assert!(validate_form(&edit, &existing, &m.rules).is_ok());
    }

    #[test]
    fn form_submit_sends_add_or_update() {
        let today = d(2026, 9, 15);
        let (mut m, rx) = model(today);
        m.screen = Screen::Day;
        m.day = Some(crate::tui::msg::DayData { day: Day { date: today, kind: DayKind::Work, entries: vec![] }, projects: vec![] });
        m.update(Msg::FormSubmit(FormData { id: None, start: "0900".into(), end: "1200".into(), project: "Alpha".into(), comment: "x".into() }));
        assert!(matches!(rx.try_recv().unwrap(), StoreCmd::AddEntry { project, .. } if project == "Alpha"));
        m.update(Msg::FormSubmit(FormData { id: Some(7), start: "0900".into(), end: "1200".into(), project: "Alpha".into(), comment: "x".into() }));
        assert!(matches!(rx.try_recv().unwrap(), StoreCmd::UpdateEntry { id: 7, .. }));
        m.update(Msg::FormSubmit(FormData { id: None, start: "zz".into(), end: "1200".into(), project: "Alpha".into(), comment: "".into() }));
        assert!(rx.try_recv().is_err());
        assert!(m.form.as_ref().unwrap().error.is_some());
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `nix develop -c cargo test tui::model`
Expected: compile errors for `validate_form`, `FormData` import, `m.form`.

- [ ] **Step 3: Implement `validate_form` and `FormState` in `model.rs`**

```rust
pub struct FormState { pub data: FormData, pub error: Option<String>, pub preview: Option<(Minutes, Minutes, Minutes)> }

pub fn validate_form(d: &FormData, existing: &[Entry], rules: &Rules) -> Result<(NaiveTime, NaiveTime, Minutes, Minutes, Minutes), String> {
    let start = parse_time(&d.start).map_err(|_| format!("start: '{}' is not a time (try 800 or 8:00)", d.start))?;
    let end = parse_time(&d.end).map_err(|_| format!("end: '{}' is not a time (try 1730 or 17:30)", d.end))?;
    if d.project.trim().is_empty() { return Err("project: required".into()); }
    check_overlap(existing, start, end, d.id).map_err(|_| "overlaps an existing entry".to_string())?;
    let this = Entry { id: d.id.unwrap_or(-1), date: existing.first().map(|e| e.date).unwrap_or_default(), start, end, project: String::new(), comment: String::new() };
    let gross_this = this.duration();
    let gross_day: Minutes = existing.iter().filter(|e| Some(e.id) != d.id).map(Entry::duration).sum::<Minutes>() + gross_this;
    let ded = deduction(gross_day, &rules.tiers);
    Ok((start, end, gross_this, ded, Minutes((gross_day - ded).0.max(0))))
}
```

Add `pub form: Option<FormState>` to `Model` (and to the test constructor: `form: None`). Model arms:

```rust
            Msg::DayAdd => self.open_form(FormData::default()),
            Msg::DayEdit => {
                if let Some(e) = self.day.as_ref().and_then(|d| d.day.entries.get(self.day_cursor)) {
                    self.open_form(FormData { id: Some(e.id), start: e.start.format("%H:%M").to_string(), end: e.end.format("%H:%M").to_string(), project: e.project.clone(), comment: e.comment.clone() });
                }
            }
            Msg::FormCancel => { self.form = None; self.focus_screen(); }
            Msg::FormSubmit(data) => {
                let Some(day) = &self.day else { return };
                if !day.day.kind.allows_entries() { self.set_status(format!("{} day: change the day type to work first", day.day.kind.display_name()), true); return; }
                match validate_form(&data, &day.day.entries, &self.rules) {
                    Err(e) => { if let Some(f) = &mut self.form { f.error = Some(e); } }
                    Ok((start, end, ..)) => {
                        let (project, comment) = (data.project.trim().to_string(), data.comment.trim().to_string());
                        match data.id {
                            None => self.worker.send(StoreCmd::AddEntry { date: day.day.date, start, end, project, comment }),
                            Some(id) => self.worker.send(StoreCmd::UpdateEntry { id, start, end, project, comment }),
                        }
                        self.form = None;
                        self.focus_screen();
                    }
                }
            }
```

`open_form`:
```rust
    fn open_form(&mut self, data: FormData) {
        let projects: Vec<String> = self.day.as_ref().map(|d| d.projects.iter().map(|p| p.name.clone()).collect()).unwrap_or_default();
        let _ = self.app.umount(&Id::Form);
        let _ = self.app.mount(Id::Form, Box::new(components::form::EntryForm::new(data.clone(), projects)), vec![]);
        self.form = Some(FormState { data, error: None, preview: None });
        self.focus(Id::Form);
    }
```

`FormSubmit` when `m.form` is `None` (tests call it directly): treat as a submit anyway; only touch `self.form` if present. In the `form_submit_sends_add_or_update` test the third submit expects `m.form...error` — so `open_form` must have been called; adjust the test to call `m.update(Msg::DayAdd)` first (mount of `Id::Form` on the test `Application` succeeds because `Application::init` works without a terminal).

- [ ] **Step 4: Implement `components/form.rs`**

```rust
use tui_realm_stdlib::Input;
use tuirealm::command::{Cmd, CmdResult, Direction, Position};
use tuirealm::component::{AppComponent, Component};
use tuirealm::event::{Event, Key, KeyEvent, KeyModifiers};
use tuirealm::props::{AttrValue, Attribute, Borders, Color, Props, QueryResult, Style};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout, Rect};
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use tuirealm::state::{State, StateValue};

use crate::tui::msg::{FormData, Msg, UserEvent};
use crate::tui::view::chrome::centered;

const LABELS: [&str; 4] = ["Start", "End", "Project", "Comment"];

pub struct EntryForm {
    props: Props,
    fields: [Input; 4],
    focus: usize,
    id: Option<i64>,
    projects: Vec<String>,
    picker_idx: usize,
}

impl EntryForm {
    pub fn new(initial: FormData, projects: Vec<String>) -> Self {
        let mk = |title: &str, value: &str, placeholder: &str| Input::default().title(title).value(value).placeholder(placeholder)
            .borders(Borders::default().modifiers(BorderType::Rounded)).invalid_style(Style::default().fg(Color::Red));
        let mut f = Self {
            props: Props::default(),
            fields: [
                mk("Start", &initial.start, "0800"),
                mk("End", &initial.end, "1730"),
                mk("Project", &initial.project, "type to filter, ↑↓ pick"),
                mk("Comment", &initial.comment, "optional"),
            ],
            focus: 0,
            id: initial.id,
            projects,
            picker_idx: 0,
        };
        f.set_focus(0);
        f
    }

    fn set_focus(&mut self, i: usize) {
        for (n, fld) in self.fields.iter_mut().enumerate() { fld.attr(Attribute::Focus, AttrValue::Flag(n == i)); }
        self.focus = i;
    }

    fn value(&self, i: usize) -> String {
        match self.fields[i].state() { State::One(StateValue::String(s)) => s, _ => String::new() }
    }

    pub fn data(&self) -> FormData {
        FormData { id: self.id, start: self.value(0), end: self.value(1), project: self.value(2), comment: self.value(3) }
    }

    fn matches(&self) -> Vec<String> {
        let q = self.value(2).to_lowercase();
        self.projects.iter().filter(|p| p.to_lowercase().contains(&q)).cloned().collect()
    }
}

impl Component for EntryForm {
    fn view(&mut self, f: &mut Frame, area: Rect) {
        let r = centered(area, 64, 16);
        f.render_widget(Clear, r);
        let outer = Block::default().borders(tuirealm::ratatui::widgets::Borders::ALL).border_type(BorderType::Rounded)
            .title(if self.id.is_some() { " Edit entry " } else { " New entry " });
        let inner = outer.inner(r);
        f.render_widget(outer, r);
        let [row1, proj, picker, comment, footer] = Layout::vertical([
            Constraint::Length(3), Constraint::Length(3), Constraint::Length(3), Constraint::Length(3), Constraint::Length(1),
        ]).areas(inner);
        let [s, e] = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(row1);
        self.fields[0].view(f, s);
        self.fields[1].view(f, e);
        self.fields[2].view(f, proj);
        let m = self.matches();
        let line: Vec<Span> = m.iter().enumerate().take(6).flat_map(|(i, p)| {
            let style = if i == self.picker_idx && self.focus == 2 { Style::default().fg(Color::Black).bg(Color::Cyan) } else { Style::default().fg(Color::DarkGray) };
            vec![Span::styled(format!(" {p} "), style), Span::raw(" ")]
        }).collect();
        f.render_widget(Paragraph::new(Line::from(line)), picker);
        self.fields[3].view(f, comment);
        let text = self.props.get(Attribute::Text).and_then(AttrValue::as_string).cloned().unwrap_or_default();
        let is_err = matches!(self.props.get(Attribute::Custom("error")), Some(AttrValue::Flag(true)));
        f.render_widget(Paragraph::new(Span::styled(text, Style::default().fg(if is_err { Color::Red } else { Color::DarkGray }))), footer);
    }
    fn query<'a>(&'a self, attr: Attribute) -> Option<QueryResult<'a>> { self.props.get_for_query(attr) }
    fn attr(&mut self, attr: Attribute, value: AttrValue) { self.props.set(attr, value); }
    fn state(&self) -> State { State::None }
    fn perform(&mut self, cmd: Cmd) -> CmdResult { CmdResult::Invalid(cmd) }
}

impl AppComponent<Msg, UserEvent> for EntryForm {
    fn on(&mut self, ev: &Event<UserEvent>) -> Option<Msg> {
        let KeyEvent { code, modifiers } = ev.as_keyboard()?.clone();
        match (code, modifiers) {
            (Key::Esc, _) => return Some(Msg::FormCancel),
            (Key::Char('s'), KeyModifiers::CONTROL) => return Some(Msg::FormSubmit(self.data())),
            (Key::Tab, _) | (Key::Down, _) if self.focus != 2 => { self.set_focus((self.focus + 1) % 4); return None; }
            (Key::BackTab, _) | (Key::Up, _) if self.focus != 2 => { self.set_focus((self.focus + 3) % 4); return None; }
            (Key::Down, _) => { self.picker_idx = (self.picker_idx + 1).min(self.matches().len().saturating_sub(1)); return None; }
            (Key::Up, _) => { self.picker_idx = self.picker_idx.saturating_sub(1); return None; }
            (Key::Tab, _) => { self.accept_pick(); self.set_focus(3); return None; }
            (Key::BackTab, _) => { self.set_focus(1); return None; }
            (Key::Enter, _) => {
                if self.focus == 2 { self.accept_pick(); self.set_focus(3); return None; }
                if self.focus == 3 { return Some(Msg::FormSubmit(self.data())); }
                self.set_focus(self.focus + 1);
                return None;
            }
            _ => {}
        }
        let cmd = match code {
            Key::Char(c) => Cmd::Type(c),
            Key::Backspace => Cmd::Delete,
            Key::Delete => Cmd::Cancel,
            Key::Left => Cmd::Move(Direction::Left),
            Key::Right => Cmd::Move(Direction::Right),
            Key::Home => Cmd::GoTo(Position::Begin),
            Key::End => Cmd::GoTo(Position::End),
            _ => Cmd::None,
        };
        self.fields[self.focus].perform(cmd);
        if self.focus == 2 { self.picker_idx = 0; }
        None
    }
}

impl EntryForm {
    fn accept_pick(&mut self) {
        let m = self.matches();
        if let Some(p) = m.get(self.picker_idx) { self.fields[2].attr(Attribute::Value, AttrValue::String(p.clone())); }
    }
}
```

Check the stdlib `Input` API in `~/.cargo/registry/src/*/tui-realm-stdlib-4.1.0/src/components/input.rs` for exact `Cmd` handling (`Cmd::Type`, `Cmd::Delete` = backspace, `Cmd::Cancel` = delete) and the `State::One(StateValue::String)` shape; adjust names to match.

Live preview: in `Model::draw` for `Screen::Day`, when `self.form.is_some()`, compute `validate_form(&form.data, ...)` (data refreshed on every key: add `Msg::FormChanged(FormData)` returned by `on` after each edit, updating `self.form.data` and `error`/`preview`), then push the footer text into the component via `self.app.attr(&Id::Form, Attribute::Text, AttrValue::String(..))` and `Attribute::Custom("error")` — do this in `update`, not in `draw` (draw takes `&self`). Then `view/day.rs::draw` ends with `m.app` not accessible… so instead render the form from `Model::view` (the `&mut self` path): after `term.draw(|f| self.draw(f))`, tui-realm's `app.view(&Id::Form, f, area)` needs `&mut self.app`; restructure `Model::view` to draw the base with `self.draw(f)` and then `self.app.view(&Id::Form, f, f.area())` inside the same closure using a split borrow (`let app = &mut self.app; let model_ref = &*self;` is not possible) — simplest: move `app` out with `std::mem::replace`-style `take` into a local for the duration of the draw, like `terminal` is taken. Implement `Model::view` as:

```rust
    pub fn view(&mut self) {
        let mut term = self.terminal.take().expect("terminal");
        let form_open = self.form.is_some();
        let _ = term.draw(|f| {
            self.draw(f);
            if form_open { let _ = self.app.view(&Id::Form, f, f.area()); }
        });
        self.terminal = Some(term);
    }
```

`self.draw(f)` borrows `&self` and `self.app.view` borrows `&mut self.app` sequentially inside the closure, which the borrow checker accepts because they are not overlapping.

- [ ] **Step 5: Run tests, manual check, commit**

Run: `nix develop -c cargo test tui` → all pass.
Manual: in the day editor press `a`, type `900`, Tab, `1230`, Tab, type `Al`, Enter (picks Alpha or creates it), type a comment, Enter → entry appears, status says "Added 09:00–12:30 Alpha". Entering `1100`–`1200` on top of it shows the red overlap message and keeps the form open.

```bash
./scripts/check.sh
git add src/tui
git commit -m "feat(tui): entry form with project picker and live validation"
```

---

### Task 17: Statistics screen

**Files:**
- Create: `src/tui/view/stats.rs` (replace stub), `src/tui/components/stats.rs` (replace stub)
- Modify: `src/tui/model.rs` (`OpenStats`, `StatsRange`), `src/tui/mod.rs` (mount `Id::Stats`)

**Interfaces:**
- Produces:
  ```rust
  pub struct StatsView { pub from: NaiveDate, pub to: NaiveDate, pub project_totals: Vec<(String, Minutes, u8)>, pub total: Minutes, pub net: Minutes, pub target: Minutes,
      pub vacation_used: u32, pub vacation_allowance: u32, pub sick: u32, pub flex: u32, pub holidays: u32, pub absences: u32, pub missing: u32 }
  pub fn range_for(kind: RangeKind, today: NaiveDate) -> (NaiveDate, NaiveDate)
  pub fn build_stats(data: &StatsData, rules: &Rules, cal: &HolidayCalendar, today: NaiveDate, allowance: u32) -> StatsView
  pub fn draw(m: &Model, f: &mut Frame, area: Rect)
  pub fn draw_stats(f: &mut Frame, area: Rect, t: &Theme, v: &StatsView, active: RangeKind)
  // components/stats.rs: StatsScreen — 1/2/3/4 → StatsRange(ThisMonth/LastMonth/Quarter/Year), ? help, Esc/q Back
  ```

- [ ] **Step 1: Write failing tests (`view/stats.rs`)**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Day, DayKind, Entry, HolidayCalendar, Minutes, Rules, default_tiers};
    use crate::tui::msg::{RangeKind, StatsData};
    use crate::tui::theme::Theme;
    use crate::tui::view::testing::{contains, render};
    use chrono::{NaiveDate, NaiveTime};

    fn d(y: i32, m: u32, dd: u32) -> NaiveDate { NaiveDate::from_ymd_opt(y, m, dd).unwrap() }

    #[test]
    fn ranges() {
        let today = d(2026, 9, 15);
        assert_eq!(range_for(RangeKind::ThisMonth, today), (d(2026, 9, 1), d(2026, 9, 30)));
        assert_eq!(range_for(RangeKind::LastMonth, today), (d(2026, 8, 1), d(2026, 8, 31)));
        assert_eq!(range_for(RangeKind::Quarter, today), (d(2026, 7, 1), d(2026, 9, 30)));
        assert_eq!(range_for(RangeKind::Year, today), (d(2026, 1, 1), d(2026, 12, 31)));
        assert_eq!(range_for(RangeKind::LastMonth, d(2026, 1, 10)), (d(2025, 12, 1), d(2025, 12, 31)));
    }

    #[test]
    fn builds_and_renders() {
        let t = |h| NaiveTime::from_hms_opt(h, 0, 0).unwrap();
        let mk = |dd, kind, entries| Day { date: d(2026, 9, dd), kind, entries };
        let e = |id, dd, s, en, p: &str| Entry { id, date: d(2026, 9, dd), start: t(s), end: t(en), project: p.into(), comment: "".into() };
        let days = vec![
            mk(1, DayKind::Work, vec![e(1, 1, 8, 16, "Alpha")]),
            mk(2, DayKind::Vacation, vec![]),
            mk(3, DayKind::Sick, vec![]),
            mk(4, DayKind::Work, vec![e(2, 4, 8, 12, "Beta")]),
            mk(7, DayKind::Flex, vec![]),
            mk(8, DayKind::Work, vec![]), // missing
        ];
        let data = StatsData { from: d(2026, 9, 1), to: d(2026, 9, 8), days, projects: vec![] };
        let rules = Rules { daily_target: Minutes(468), tiers: default_tiers(), start_date: d(2026, 1, 1), initial_balance: Minutes::ZERO };
        let v = build_stats(&data, &rules, &HolidayCalendar::default(), d(2026, 9, 15), 30);
        assert_eq!(v.project_totals[0], ("Alpha".to_string(), Minutes(480), 0));
        assert_eq!(v.total, Minutes(720));
        assert_eq!((v.vacation_used, v.sick, v.flex, v.missing), (1, 1, 1, 1));
        let rows = render(100, 24, |f| draw_stats(f, f.area(), &Theme::dark(), &v, RangeKind::ThisMonth));
        assert!(contains(&rows, "2026-09-01 → 2026-09-08"));
        assert!(contains(&rows, "Alpha"));
        assert!(contains(&rows, "66.7%"));
        assert!(contains(&rows, "vacation"));
        assert!(contains(&rows, "1 / 30"));
        assert!(contains(&rows, "[1] this month"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `nix develop -c cargo test view::stats` → compile error.

- [ ] **Step 3: Implement `view/stats.rs`**

```rust
use chrono::{Datelike, NaiveDate};
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::{Constraint, Layout, Rect};
use tuirealm::ratatui::style::{Modifier, Style};
use tuirealm::ratatui::text::{Line, Span};
use tuirealm::ratatui::widgets::Paragraph;

use super::{bar, block, minutes_span};
use crate::core::{DayKind, HolidayCalendar, Minutes, Rules, TodayCtx, day_stats};
use crate::tui::model::Model;
use crate::tui::msg::{RangeKind, StatsData};
use crate::tui::theme::Theme;
use crate::tui::worker::month_range;

pub struct StatsView {
    pub from: NaiveDate, pub to: NaiveDate,
    pub project_totals: Vec<(String, Minutes, u8)>, pub total: Minutes, pub net: Minutes, pub target: Minutes,
    pub vacation_used: u32, pub vacation_allowance: u32, pub sick: u32, pub flex: u32, pub holidays: u32, pub absences: u32, pub missing: u32,
}

pub fn range_for(kind: RangeKind, today: NaiveDate) -> (NaiveDate, NaiveDate) {
    match kind {
        RangeKind::ThisMonth => month_range(today.year(), today.month()),
        RangeKind::LastMonth => { let (y, m) = if today.month() == 1 { (today.year() - 1, 12) } else { (today.year(), today.month() - 1) }; month_range(y, m) }
        RangeKind::Quarter => { let q0 = (today.month() - 1) / 3 * 3 + 1; (month_range(today.year(), q0).0, month_range(today.year(), q0 + 2).1) }
        RangeKind::Year => (NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap(), NaiveDate::from_ymd_opt(today.year(), 12, 31).unwrap()),
    }
}

pub fn build_stats(data: &StatsData, rules: &Rules, cal: &HolidayCalendar, today: NaiveDate, allowance: u32) -> StatsView {
    let ctx = TodayCtx { today, clocked_in: false };
    let mut totals: std::collections::BTreeMap<String, Minutes> = Default::default();
    let mut v = StatsView { from: data.from, to: data.to, project_totals: vec![], total: Minutes::ZERO, net: Minutes::ZERO, target: Minutes::ZERO, vacation_used: 0, vacation_allowance: allowance, sick: 0, flex: 0, holidays: 0, absences: 0, missing: 0 };
    for day in &data.days {
        let s = day_stats(day, rules, cal, &ctx);
        v.net += s.net; v.target += s.target;
        if s.missing { v.missing += 1; }
        for e in &day.entries { *totals.entry(e.project.clone()).or_default() += e.duration(); v.total += e.duration(); }
        if crate::core::is_working_day(day.date) {
            match day.kind { DayKind::Vacation => v.vacation_used += 1, DayKind::Sick => v.sick += 1, DayKind::Flex => v.flex += 1, DayKind::Holiday => v.holidays += 1, DayKind::Absence { .. } => v.absences += 1, DayKind::Work => {} }
        }
    }
    v.project_totals = totals.into_iter().map(|(n, m)| { let idx = data.projects.iter().find(|p| p.name == n).map(|p| p.color_index).unwrap_or(0); (n, m, idx) }).collect();
    v.project_totals.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

pub fn draw(m: &Model, f: &mut Frame, area: Rect) {
    let Some(data) = &m.stats else { f.render_widget(Paragraph::new("loading…").style(Style::default().fg(m.theme.muted)), area); return; };
    let v = build_stats(data, &m.rules, &m.cal, m.today, m.vacation_allowance);
    draw_stats(f, area, &m.theme, &v, m.stats_range);
}

pub fn draw_stats(f: &mut Frame, area: Rect, t: &Theme, v: &StatsView, active: RangeKind) {
    let [sel, proj, kinds] = Layout::vertical([Constraint::Length(3), Constraint::Min(6), Constraint::Length(6)]).areas(area);
    let mut spans = Vec::new();
    for (k, key, label) in [(RangeKind::ThisMonth, "1", "this month"), (RangeKind::LastMonth, "2", "last month"), (RangeKind::Quarter, "3", "quarter"), (RangeKind::Year, "4", "year")] {
        let style = if k == active { Style::default().fg(t.accent).add_modifier(Modifier::BOLD) } else { Style::default().fg(t.muted) };
        spans.push(Span::styled(format!("[{key}] {label}   "), style));
    }
    spans.push(Span::styled(format!("{} → {}", v.from, v.to), Style::default().fg(t.text)));
    f.render_widget(Paragraph::new(Line::from(spans)).block(block(t, Some("Range"))), sel);

    let total = v.total.0.max(1) as f64;
    let bar_w = proj.width.saturating_sub(2 + 20 + 2 + 9 + 8 + 2).max(10);
    let mut lines: Vec<Line> = v.project_totals.iter().map(|(n, m, idx)| {
        let frac = m.0 as f64 / total;
        let mut l = vec![Span::styled(format!("{:<20}", n), Style::default().fg(t.project_color(*idx)))];
        l.extend(bar(frac, bar_w, t.project_color(*idx), t).spans);
        l.push(Span::raw(format!("  {:>8}  {:>5.1}%", m.hhmm(), frac * 100.0)));
        Line::from(l)
    }).collect();
    if lines.is_empty() { lines.push(Line::from(Span::styled("no entries in range", Style::default().fg(t.muted)))); }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![Span::styled("worked ", Style::default().fg(t.muted)), Span::raw(v.total.hhmm()), Span::styled("   net ", Style::default().fg(t.muted)), Span::raw(v.net.hhmm()), Span::styled("   target ", Style::default().fg(t.muted)), Span::raw(v.target.hhmm()), Span::styled("   balance ", Style::default().fg(t.muted)), minutes_span(v.net - v.target, t)]));
    f.render_widget(Paragraph::new(lines).block(block(t, Some("Projects"))), proj);

    let kl = vec![
        Line::from(vec![Span::styled("vacation  ", Style::default().fg(t.chip_vacation)), Span::raw(format!("{} / {} used, {} left", v.vacation_used, v.vacation_allowance, v.vacation_allowance.saturating_sub(v.vacation_used)))]),
        Line::from(vec![Span::styled("sick      ", Style::default().fg(t.chip_sick)), Span::raw(v.sick.to_string()), Span::styled("    flex  ", Style::default().fg(t.chip_flex)), Span::raw(v.flex.to_string()), Span::styled("    holidays  ", Style::default().fg(t.chip_holiday)), Span::raw(v.holidays.to_string()), Span::styled("    absences  ", Style::default().fg(t.chip_absence)), Span::raw(v.absences.to_string())]),
        Line::from(vec![Span::styled("missing   ", Style::default().fg(t.negative)), Span::raw(v.missing.to_string())]),
    ];
    f.render_widget(Paragraph::new(kl).block(block(t, Some("Days"))), kinds);
}
```

The test expects `"1 / 30"`; the rendered text is `1 / 30 used, 29 left`, which contains it. The `66.7%` expectation is Alpha 480 / total 720.

- [ ] **Step 4: Component and model arms**

`components/stats.rs`: `StatsScreen` mapping `1`..`4` → `Msg::StatsRange(..)`, `?` → `ToggleHelp`, `Esc`/`q` → `Back`, `Ctrl+C` → `Quit`.

Model: add `pub stats_range: RangeKind` (default `ThisMonth`; add to test constructor) and arms:

```rust
            Msg::OpenStats => { self.screen = Screen::Stats; self.stats = None; self.focus(Id::Stats); self.load_stats(); }
            Msg::StatsRange(r) => { self.stats_range = r; self.stats = None; self.load_stats(); }
```
with
```rust
    fn load_stats(&self) { let (from, to) = super::view::stats::range_for(self.stats_range, self.today); self.worker.send(StoreCmd::LoadStats { from, to }); }
```

Mount `Id::Stats` in `tui::run`.

- [ ] **Step 5: Run tests, manual check, commit**

Run: `nix develop -c cargo test tui` → all pass. Manual: `s` opens stats with this month's bars; `4` switches to the year; `Esc` returns.

```bash
./scripts/check.sh
git add src/tui
git commit -m "feat(tui): statistics screen with range selector"
```

---

### Task 18: Full-screen snapshot tests, README, flake package build

**Files:**
- Create: `tests/snapshots.rs`, `README.md`
- Modify: `flake.nix` (verify `nix build` works), `Cargo.toml` (nothing unless build needs it)

**Interfaces:**
- Consumes: `Model::draw`, `model::testing::model`, view builders.

- [ ] **Step 1: Write full-screen snapshot tests `tests/snapshots.rs`**

These use the library's public API (`tk::tui::model::testing` must be `pub` under `#[cfg(test)]` — integration tests cannot see `cfg(test)` items, so move the test constructor to `pub mod testing` gated by a cargo feature `test-support` **or** simply make `testing` a normal `pub mod` with `#[doc(hidden)]`). Use the latter.

```rust
use chrono::NaiveDate;
use tk::core::{Day, DayKind, Entry};
use tk::tui::model::testing::{model, month_data};
use tuirealm::ratatui::backend::TestBackend;
use tuirealm::ratatui::Terminal;

fn rows(w: u16, h: u16, draw: impl FnOnce(&mut tuirealm::ratatui::Frame)) -> Vec<String> {
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(draw).unwrap();
    let buf = term.backend().buffer().clone();
    (0..h).map(|y| (0..w).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string()).collect()
}

#[test]
fn month_screen_100x30_and_80x24() {
    let today = NaiveDate::from_ymd_opt(2026, 9, 15).unwrap();
    let (mut m, _rx) = model(today);
    let t = |h| chrono::NaiveTime::from_hms_opt(h, 0, 0).unwrap();
    let days: Vec<Day> = (1..=30).map(|dd| {
        let date = NaiveDate::from_ymd_opt(2026, 9, dd).unwrap();
        let entries = if dd == 14 { vec![Entry { id: 1, date, start: t(9), end: t(17), project: "Alpha".into(), comment: "snapshot".into() }] } else { vec![] };
        Day { date, kind: DayKind::Work, entries }
    }).collect();
    m.month = Some(month_data(today, days, None));
    let big = rows(100, 30, |f| m.draw(f));
    assert!(big.iter().any(|r| r.contains("SEPTEMBER 2026")));
    assert!(big.iter().any(|r| r.contains("snapshot")));
    assert!(big.iter().any(|r| r.contains("q quit")));
    let small = rows(80, 24, |f| m.draw(f));
    assert!(small.iter().any(|r| r.contains("Mon 14")));
    assert!(!small.iter().any(|r| r.contains("snapshot"))); // comment hidden < 90 cols
    let tiny = rows(60, 20, |f| m.draw(f));
    assert!(tiny.iter().any(|r| r.contains("too small")));
}
```

- [ ] **Step 2: Run, fix visibility, commit**

Run: `nix develop -c cargo test --test snapshots` → 1 passed.

- [ ] **Step 3: Write `README.md`**

Sections: what it is (two sentences), install (`nix build` / `cargo install --path .`), first run (creates `~/.local/share/tk/config.toml`), the config reference copied from `DEFAULT_TOML` with one line per field, CLI table (every command from Task 10), TUI key map (every key from Tasks 12–17), portability (`copy the tk folder`; `tk backup`), holidays note (Saxony/Dresden, no Fronleichnam, `extra_holidays`), development (`./scripts/check.sh`).

- [ ] **Step 4: Verify the Nix package builds**

Run: `nix build` then `./result/bin/tk --help`.
Expected: help text. If `cargoLock.lockFile` complains, commit `Cargo.lock` first (it must be tracked).

- [ ] **Step 5: Final check and commit**

Run: `./scripts/check.sh`

```bash
git add -A
git commit -m "docs: README; test: full-screen snapshots; build: verify nix package"
```

---

## Self-review against the spec

- §5 rules: Tasks 3, 5, 6, 7 (each rule has a named test). Today rule: `today_rules` test. Missing: `past_weekday_without_entries_is_missing`.
- §6 home dir, schema, config: Tasks 8, 9. Backup/export: Task 10.
- §7 runtime: Task 12 (tokio + ticker + store worker + panic hook). Errors: `StoreReply::Failed` → status bar; CLI exit codes 1/2 in Task 10.
- §8 CLI: Task 10 covers every command; DATE forms in Task 4.
- §9 screens: month (13, 14), day editor (15, 16), stats (17), shared chrome (12), help/confirm (14).
- §10 visual: theme roles, palette, truecolor detection and 256 fallback, overrides (11); chips not banners (13); responsive rules (13, 18); minimum size notice (12).
- §11 tests: per-module tests in each task; snapshot tests (13, 15, 17, 18); CLI `assert_cmd` (10).
- §12 packaging: flake (1, 18), README (18).
- Deferred to a follow-up plan (explicitly out of v1 per spec §2): year overview, CSV import, explicit breaks.

Type consistency notes for executors: `Minutes` is `Copy`; `DayKind` is not (`clone()` it); `Model::draw` takes `&self`, everything under `view/` takes `&Model` or plain data; the only `&mut` drawing path is `Model::view` for the form overlay.
