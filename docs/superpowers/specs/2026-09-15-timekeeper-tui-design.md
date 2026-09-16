# Timekeeper (`tk`) — Design Spec

Date: 2026-09-15
Status: approved design, pre-implementation
Replaces: the Python `~/Timekeeping/time` script (analysed 2026-09-15; not migrated)

## 1. Purpose

A Rust terminal application for tracking a German-style flexitime (Gleitzeit)
balance. It records work entries per day, applies tiered break deductions,
knows vacation, flex, holiday, sick and absence days, and shows the running
balance in a full-screen TUI. Short CLI commands cover the daily habits
(clock in, clock out, status) without opening the UI.

Single user, single device, portable by copying one directory.

## 2. Non-goals (v1)

- No import of the old `timesheet.csv`. The old data is not maintained.
- No year overview screen (deferred).
- No multi-user, sync, or server component.
- No explicit break recording; breaks are always the configured tiers.
- No Windows support target; Linux first, macOS should work untested.

## 3. Technology

| Concern       | Choice                                         |
|---------------|------------------------------------------------|
| Language      | Rust, edition 2024, stable toolchain           |
| TUI           | `ratatui` + `crossterm` via `tuirealm` (tui-realm) and `tui-realm-stdlib` |
| Async runtime | `tokio` (multi-thread runtime, minimal usage; see §7.1) |
| CLI parsing   | `clap` (derive)                                |
| Storage       | `rusqlite` with `bundled` feature (SQLite compiled in) |
| Config        | `serde` + `toml`                               |
| Dates         | `chrono`                                       |
| Errors        | `thiserror` for library errors, `anyhow` at the binary edge |
| Paths         | `directories` for XDG resolution               |
| Text input    | `tui-realm-stdlib` Input component             |
| Testing       | built-in test harness, `tempfile` for DB tests, ratatui `TestBackend` for snapshots |
| Packaging     | Cargo binary `tk`; Nix flake with package + dev shell |

## 4. Project structure

Single crate, binary name `tk`, repository `~/timekeeper`.

```
src/
  main.rs          entry: parse CLI, resolve home dir, dispatch
  core/
    mod.rs
    types.rs       Date-free value types: Minutes, DayKind, Entry, Day, Project
    breaks.rs      tiered break deduction
    balance.rs     target, net, per-day and running balance
    holidays.rs    Saxony (Dresden) public holidays + extra dates
    time_parse.rs  "800" / "0800" / "8:00" / "1730" → NaiveTime
  store/
    mod.rs         Store struct, open(), migrations
    schema.sql     versioned schema
    days.rs        day kind queries
    entries.rs     entry CRUD
    projects.rs    project CRUD
    session.rs     clock-in state
  config/
    mod.rs         Config struct, load/validate/write default
  cli/
    mod.rs         clap definitions
    commands.rs    in, out, status, add, day, backup, export, projects
  tui/
    mod.rs         run(): tokio runtime, tui-realm Application, event loop
    model.rs       Model (all UI state), Msg enum, update()
    ids.rs         component Id enum
    theme.rs       color roles, palette, capability detection
    screens/
      month.rs
      day_editor.rs
      stats.rs
    components/
      title_bar.rs
      status_bar.rs
      key_hints.rs
      confirm.rs
      help_overlay.rs
      entry_form.rs
      project_picker.rs
```

Rules:

- `core` has no I/O and no dependency on `store`, `config`, or `tui`.
- `store` is the only module that imports `rusqlite`.
- `tui` and `cli` both depend on `core`, `store`, `config`; never on each other.

## 5. Domain model

### 5.1 Types (core)

```rust
pub struct Minutes(pub i32);               // signed, arithmetic helpers, Display "+HH:MM"/"-HH:MM"

pub enum DayKind {
    Work,                                  // default; may have entries
    Vacation,
    Flex,                                  // Gleitzeit-Tag: full target deducted, no entries
    Holiday,                               // computed or extra; may be set manually
    Sick,
    Absence { label: String },             // training, business trip, ...
}

pub struct Entry {
    pub id: i64,
    pub date: NaiveDate,
    pub start: NaiveTime,
    pub end: NaiveTime,                    // end <= start means crosses midnight
    pub project: String,
    pub comment: String,
    pub break_share: Option<Minutes>,      // explicit share of the session's deduction
}

pub struct Day {
    pub date: NaiveDate,
    pub kind: DayKind,
    pub entries: Vec<Entry>,               // non-empty only when kind == Work
}

pub struct Project {
    pub id: i64,
    pub name: String,
    pub color_index: u8,                   // 0..10, stable
    pub archived: bool,
}
```

### 5.2 Rules

**Working days.** Monday–Friday are working days. Saturday and Sunday never
carry a target and never show as missing.

**Target.** `daily_target_minutes` applies on a working day when the kind is
`Work` or `Flex`. Vacation, Holiday, Sick and Absence carry zero target.

**Gross.** Sum of entry durations for the day. An entry whose `end <= start`
wraps to the next day (duration = end + 24h − start).

**Break deduction.** The entries of a day are grouped into *sessions*:
consecutive entries whose intervals touch or overlap (no gap) form one session,
and any gap of at least one minute starts a new one. A back-to-back project
switch therefore stays a single session, while a real pause of any length
splits the day. `break_tiers` is a list of `{after_minutes, deduct_minutes}`
sorted ascending; the tier deduction for a length is `deduct_minutes` of the
last tier whose `after_minutes` is strictly below it, else 0. That deduction is
applied to **each session's own length**, and the day's deduction is the sum.
Default tiers: over 180 → 18, over 360 → 48. So 08–12 plus 12–17 is one
nine-hour session and loses 48; 08–12 plus 12:45–17 is 4:00 plus 4:15 and loses
18 + 18 = 36; 08–14:30 plus 14:31–17 is 6:30 plus 2:29 and loses 48 + 0. A day
without entries is never charged, and an entry that crosses midnight is one
session of its normalised length. While the user is clocked in, the running
session counts as one more interval, so it merges with the entry it continues
and stands alone after a pause.

**Net.** `gross − deduction`, never negative.

**Net per entry.** A session's deduction (see "Break deduction") has to be
carried by the entries of that session, and two rules decide which:

1. *Explicit shares.* An entry's `break_share`, when set, is the minutes it
   pays, capped by its own gross. If the explicit shares of a session come to
   more than the session's deduction they are scaled down in proportion to each
   other, to whole minutes by the largest-remainder method (ties to the earlier
   entry), so they add up to the deduction exactly and nothing is left for the
   unassigned entries.
2. *The default: the project worked last.* Whatever the deduction still needs
   falls on the **last unassigned** entry of the session as far as its gross
   allows, then on the one before it, and so on. That is where a break actually
   lands: the clock stops, and the project being worked at that moment pays for
   it. If every entry of the session is assigned and the shares fall short, the
   remainder walks the same way from the last entry backwards — an explicit
   share is a floor in that case, not a ceiling, because the day's net may not
   disagree with the day's deduction.

An entry's net is its gross minus its share. Project totals, the month summary,
the statistics and the day editor use entry nets; pauses never count as project
time. With the default tiers, 08–12 on Alpha plus 12–17 on Beta is one nine-hour
session losing 48 minutes, and with no share assigned all 48 come off Beta: Alpha
nets 4:00, Beta 4:12 and the day 8:12. Putting 30 minutes on Alpha leaves the
other 18 on Beta: 3:30 and 4:42. A session of one entry hands it the whole
deduction; two sessions are settled independently, and a share never pays for
another session's break. A session never deducts more than was worked in it, so
no entry's net is negative and the day's net is always the sum of its entries'
nets.

**Day balance.** `net − target`. A Flex day therefore contributes `−target`.
A partial flex (leaving early) is simply a short Work day.

**Today.** Today's target is included only when (a) the user is not clocked
in and at least one entry exists, or (b) the local time is past 23:59 (i.e.
the day is over). While clocked in, the running entry's provisional net is
shown separately as "running" and not added to the balance.

**Running balance.** `initial_balance_minutes` plus the sum of day balances
from `start_date` through yesterday, plus today per the rule above. Days
before `start_date` are ignored entirely.

**Missing entry.** A past working day with kind `Work` and no entries. Shown
in red; counts as `−target`. This is intentional so forgotten days are
visible in the balance.

**Overlap.** Two entries on the same day may not overlap in time (after
midnight normalisation). Insert/update is rejected with
`CoreError::Overlap`.

**Holidays.** Computed for every year on demand for Saxony as applicable in
Dresden:

| Holiday                | Rule                                    |
|------------------------|-----------------------------------------|
| Neujahr                | 1 Jan                                   |
| Karfreitag             | Easter − 2                              |
| Ostermontag            | Easter + 1                              |
| Tag der Arbeit         | 1 May                                   |
| Christi Himmelfahrt    | Easter + 39                             |
| Pfingstmontag          | Easter + 50                             |
| Tag der Deutschen Einheit | 3 Oct                                |
| Reformationstag        | 31 Oct                                  |
| Buß- und Bettag        | Wednesday before 23 Nov                 |
| 1. Weihnachtstag       | 25 Dec                                  |
| 2. Weihnachtstag       | 26 Dec                                  |

Fronleichnam is **not** included (Dresden is not in a designated Sorbian
Catholic municipality). Easter uses the Gregorian computus (Meeus/Jones/
Butcher). `extra_holidays` from config are added. A computed holiday that
falls on a weekend has no effect. A day's stored kind, if any, overrides the
computed holiday (e.g. the user worked on a holiday and set it to Work).

**Vacation allowance.** `vacation_days_per_year`; remaining = allowance −
count of Vacation working days in the calendar year. No carry-over in v1.

## 6. Storage and configuration

### 6.1 Home directory

Resolution order: `--home <dir>` flag, `TK_HOME` env var, else
`$XDG_DATA_HOME/tk` (default `~/.local/share/tk`). Created on first run.
Contents:

```
tk.db          SQLite database (WAL mode; -wal/-shm are transient)
config.toml    user configuration, written with commented defaults on first run
backups/       created by `tk backup`
```

Portability = copy this directory.

### 6.2 Schema (v2)

```sql
CREATE TABLE meta      (key TEXT PRIMARY KEY, value TEXT NOT NULL);   -- schema_version
CREATE TABLE projects  (id INTEGER PRIMARY KEY, name TEXT UNIQUE NOT NULL,
                        color_index INTEGER NOT NULL, archived INTEGER NOT NULL DEFAULT 0);
CREATE TABLE days      (date TEXT PRIMARY KEY, kind TEXT NOT NULL, label TEXT);
CREATE TABLE entries   (id INTEGER PRIMARY KEY, date TEXT NOT NULL,
                        start_min INTEGER NOT NULL, end_min INTEGER NOT NULL,
                        project_id INTEGER NOT NULL REFERENCES projects(id),
                        comment TEXT NOT NULL DEFAULT '',
                        break_share INTEGER);            -- v2; NULL = unassigned
CREATE INDEX entries_date ON entries(date);
CREATE TABLE session   (id INTEGER PRIMARY KEY CHECK (id = 1),
                        date TEXT NOT NULL, start_min INTEGER NOT NULL,
                        project_id INTEGER,
                        state TEXT NOT NULL DEFAULT 'working');  -- v2; 'working' | 'break'
```

Dates are ISO `YYYY-MM-DD` text. Times are minutes since midnight.
Migrations are forward-only, numbered, applied in a transaction at open.
A `days` row exists only when the kind is not `Work`; deleting it resets the
day to `Work`.

**v1 → v2.** Both v2 columns are added to tables that already exist, so the
migration is two `ALTER TABLE … ADD COLUMN`s and the new `schema_version`, in
one transaction: every v1 row is kept, every entry reads as unassigned and
every session as `working`. A fresh database is created at v2 directly. A
version this build does not know is still refused with the database untouched.

**Break state.** While `state = 'break'` the session row holds the minute the
break began and the project to come back to; the work before it is already an
entry. Resuming moves `date`/`start_min` to now and the state back to
`working`; clocking out on a break clears the row and books nothing.

### 6.3 Config file

```toml
# tk configuration — edit and restart tk
start_date = "2026-01-01"          # balance is computed from this date
initial_balance_minutes = 0        # carried-over balance at start_date
daily_target_minutes = 468         # 7:48
vacation_days_per_year = 30
week_starts_on = "monday"          # display only
theme = "dark"                     # "dark" | "light" | "purple"
hours_format = "hm"                # "hm" (07:48) | "decimal" (7.80h)
extra_holidays = []                # e.g. ["2026-12-24", "2026-12-31"]

[[break_tiers]]                    # ascending; last matching tier applies
after_minutes = 180
deduct_minutes = 18

[[break_tiers]]
after_minutes = 360
deduct_minutes = 48

[theme_overrides]                  # optional; any role may be set to "#rrggbb" or a named color
# positive = "#a6e3a1"
```

Validation at load: tiers ascending and non-negative, target > 0, start_date
parses, theme value known. A validation error prints the field and line and
exits 2. `break_gap_minutes`, which older versions used to cancel a day's
deduction, is still accepted and ignored so that an existing config file keeps
loading under `deny_unknown_fields`.

## 7. Application architecture

### 7.1 Runtime

`main` parses the CLI. For all commands except the bare `tk`, it opens the
store synchronously and runs the command; no tokio. For `tk` it builds a
tokio multi-thread runtime and calls `tui::run`.

Inside the TUI, tokio hosts exactly two tasks:

1. **Ticker**: sends `Msg::Tick` every second (drives the live clock-in
   timer and midnight rollover).
2. **Store worker**: owns the `Store`, receives `StoreCmd` on an mpsc
   channel, executes, and replies with `Msg::StoreResult(...)`. The UI
   thread never blocks on SQLite.

If tokio turns out to add friction during implementation, the two tasks may
be plain threads; the message protocol stays identical.

### 7.2 tui-realm wiring

- `Application<Id, Msg, NoUserEvent>` with the crossterm event listener.
- `Id` enum: one variant per mounted component (`MonthTable`, `TitleBar`,
  `StatusBar`, `KeyHints`, `DayKindSelect`, `EntryTable`, `EntryForm*`,
  `ProjectPicker`, `StatsRange`, `StatsProjects`, `StatsDayKinds`,
  `Confirm`, `Help`).
- `Model` holds: current `Screen` enum, selected date, month cache
  (computed `Vec<DayRow>` plus totals), clock-in state, pending confirm,
  last status message, theme.
- `Msg` enum covers navigation, edits, store results, tick, and errors.
- `update(&mut Model, Msg) -> Option<Msg>` is pure over the model; store
  side effects are sent as `StoreCmd`.
- Screens are an enum; `Esc` always returns to the month view (or closes an
  overlay). Only the active screen's components are mounted.

### 7.3 Error handling

- `core::CoreError` (Overlap, InvalidTime, InvalidRange).
- `store::StoreError` (Sqlite, Migration, NotFound, Constraint).
- `config::ConfigError` (Io, Parse, Validation { field, reason }).
- CLI: errors print one line to stderr, exit codes 1 (runtime), 2 (config/usage).
- TUI: any error becomes `Msg::Error(String)` and is shown in the status bar
  in the `negative` role for 5 seconds; the app never panics on bad data.
  Panics are caught by a hook that restores the terminal before printing.

## 8. CLI

```
tk                       open the TUI
tk in                    clock in now (error if already clocked in unless --force)
tk out [-p PROJECT] [-m COMMENT]
                         clock out, create entry; project defaults to the last used one
tk status                one line: "⏱ 03:41 (in 08:12) · today +02:10 · balance +12:30"
                         exit 0; prints "not clocked in" variant otherwise
tk add DATE START-END -p PROJECT [-m COMMENT]
                         e.g. tk add 2026-09-14 0900-1530 -p Alpha -m "note"
tk day DATE KIND [--to DATE] [--label TEXT]
                         KIND: work | vacation | flex | holiday | sick | absence
tk projects [list | add NAME | archive NAME | rename OLD NEW]
tk backup                copy tk.db to backups/tk-YYYYMMDD-HHMMSS.db
tk export [--format csv|json] [--from DATE] [--to DATE] [-o FILE]
tk --home DIR ...        override home directory
```

DATE accepts `YYYY-MM-DD`, `today`, `yesterday`, or a signed offset like
`-1`.

## 9. TUI screens

### 9.1 Month view (home)

Layout, top to bottom: title bar, month table, month summary box, key hints.

Title bar: month name and year left; running balance, live clock-in timer,
and vacation remaining right. While a break is on, the timer reads
`☕ on break 00:12 (since 12:03) · Alpha` in the `warning` role instead: what
counts up is the pause, and the project named is the one waiting.

Table columns: Day, Project, Start, End, Gross, Net, Comment. Comment
flexes; others fixed. Rows:

- Work day with entries: one row per entry; day label only on the first, but
  gross and net on every one — the net is that entry's own share (§5.2 "Net per
  entry"), and the day's net is read off the week footer and the summary.
- Work day, past, no entries: red "missing" chip across the project column.
- Work day, future or today without entries: dimmed dots.
- Vacation / Flex / Holiday / Sick / Absence: one row, colored kind chip in
  the project column, label in comment (holiday name, absence label).
- Saturday + Sunday collapse into one muted "weekend" row; if either has
  entries, they render normally instead.
- After each Sunday (or month end): a right-aligned week footer with
  `KW nn  <week balance>  → <running balance>`.

Summary box: one bar per project (net hours, share), then
`target  net  month balance`, then day-kind counts for the month. The project
bars are net time, so they add up to the summary's own `net`.

Keys: `↑↓`/`jk` day, `[`/`]` month, `t` today, `Enter` day editor,
`s` statistics, `i`/`o` clock in/out, `b` take a break, `v` `f` `x` `p` set
kind (vacation, flex, sick, public holiday) on the selected day with confirm,
`w` reset to work, `?` help, `q` quit. Kind changes on a day with entries are
refused with a status message.

`b` books the work so far and pauses the clock on the same project; `i` then
comes back to work (picker titled `Resume — currently NAME`, that project
offered first) and `o` ends the break without booking anything more. `b`
without a clock running, or on a break already, is a status message. The hint
row has no room for `b` at 80 columns, so it is listed in `?` only.

### 9.2 Day editor

Opened on a date. Layout: title (date, weekday, holiday name if any),
day-kind selector (radio style), entry table, key hints.

Entry table columns: Start, End, Gross, Net share, Break share, Project,
Comment. `a` add, `e` edit, `d` delete (confirm), `b` break split, `Esc` back.

Break split (overlay), `Break split — DATE`: one field per entry of the
selected entry's session, labelled `HH:MM–HH:MM  project  (gross)`, holding
that entry's share of the session's deduction **in minutes**. An empty field
is "unassigned" and shows the default share as its placeholder. The footer
counts `assigned X / D` and refuses a save that assigns more than the session
loses, or more to an entry than its gross (see 5.2 "Net per entry"). Enter or
`Ctrl+S` saves every field in one write, `Esc` cancels. It opens on `b`, and
by itself after a clock-out or a break whose session spans more than one
project and loses a deduction; after an entry is saved in the day editor the
same case only puts `break on last project · press b to split` in the status
bar.

Entry form (overlay): Start, End, Project (picker, `Tab` to open, type to
filter, `n` to create new), Comment. Live footer shows gross, day break
deduction, day net. Time inputs accept `800`, `0800`, `8:00`, `17:30`.
Validation inline under the field: bad time, end before start when not
intended (asks "crosses midnight?"), overlap with existing entry.

### 9.3 Statistics

Range selector: This month, Last month, Quarter, Year. (A Custom range with
two date inputs was planned for v1 but deferred to a follow-up release.) Body:
the balance chart, one bar per period of the range, footed by `total`, `best`
and `worst` and then by `net`, `target` and `balance` — the only place the
target and the balance are shown, because they are about the day and not about
any project; below it the per-project table with colored bars, net hours, share
and a `net` total, which shows time on projects and nothing else; below that the
day-kind table: vacation used/remaining, sick, flex, holidays, absences,
missing days. `Esc` back.

### 9.4 Shared components

- Status bar: last message or error, auto-clears.
- Key hints bar: context-sensitive, always visible.
- Confirm dialog: yes/no, `y`/`n`/`Esc`.
- Help overlay: all keys for the current screen.

## 10. Visual design

- **Color roles** in `theme.rs`: `positive`, `negative`, `warning`,
  `accent` (selection, today), `muted` (weekends, borders, hints), `text`,
  `chip_vacation`, `chip_flex`, `chip_holiday`, `chip_sick`,
  `chip_absence`. Widgets use roles only.
- **Project palette**: 10 colors chosen for contrast on dark and light
  backgrounds (a Catppuccin-like set for dark, a Solarized-light-like set
  for light). `color_index` is assigned round-robin at project creation and
  stored, so colors are stable.
- **Capability**: truecolor if `COLORTERM=truecolor|24bit`, else 256-color
  approximations; never relies on the terminal's default 16 colors for
  meaning.
- **Typography**: rounded borders, one border style, right-aligned numbers,
  right-justified numbers, `+HH:MM`/`-HH:MM` everywhere, one title bar and
  one hint bar. Day kinds are colored label chips, not spaced-out banners.
- **Responsive**: comment column hides below 90 columns; summary box stacks
  below 100. Minimum supported size 80×24; smaller shows a "terminal too
  small" notice.
- **Overrides**: `[theme_overrides]` in config replaces any role color.

## 11. Testing

- `core`: unit tests for every break-tier boundary (exactly at
  `after_minutes` is not over), midnight-crossing durations, overlap
  detection including wrap-around, target rules per kind and weekday,
  running balance over a mixed fortnight, today rules, Saxony holidays
  2024–2030 against a fixture of known dates, Buß- und Bettag for each year,
  time parsing table.
- `store`: integration tests on a `tempfile` database: migrations from
  empty, CRUD, overlap constraint enforced through the store, session
  lifecycle, export round-trip.
- `config`: parse defaults, each validation error, overrides.
- `tui`: snapshot tests rendering each screen into a ratatui `TestBackend`
  buffer at 100×30 and 80×24 with a fixed fixture month; `update()` unit
  tests for navigation and edit flows without a terminal.
- `cli`: `assert_cmd` tests for `in`/`out`/`status`/`add`/`day` against a
  temp `--home`.

CI target: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`.

## 12. Packaging

- `Cargo.toml` binary `tk`, release profile with LTO and `strip = true`.
- `flake.nix`: `packages.default` via `rustPlatform.buildRustPackage`,
  `devShells.default` with toolchain, `rust-analyzer`, `sqlite`.
- `README.md`: install, first run, config reference, key map.

## 13. Delivery order (for the implementation plan)

1. Crate skeleton, config, store with migrations, core types + balance +
   breaks + holidays with tests.
2. CLI commands (`in`, `out`, `status`, `add`, `day`, `projects`,
   `backup`, `export`).
3. TUI shell: runtime, theme, title/status/hints, month view read-only.
4. Day editor with entry form and project picker.
5. Clock in/out and day-kind actions from the month view.
6. Statistics screen.
7. Responsiveness, help overlay, snapshot tests, Nix flake, README.
