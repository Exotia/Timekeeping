# tk import / export Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `tk import` to the CLI, and `E` (export) / `I` (import) keys to the TUI month view, sharing one implementation.

**Architecture:** A pure parser in `core` turns CSV or JSON text into `ImportRow`s. `Store::import_entries` applies them inside one transaction, reusing `add_entry_locked` so overlap, range and project rules come for free; `dry_run` rolls that transaction back instead of committing. The CLI and the TUI worker both call it, the way both already share `write_backup`.

**Tech Stack:** Rust 2024, rusqlite, clap, tuirealm, thiserror, assert_cmd + predicates + tempfile for tests.

**Spec:** No separate spec document — this plan was approved conversationally. The design it implements is the "Design" section below.

## Design

Three input formats, detected from the file itself, never from a flag:

| Source | Header |
| --- | --- |
| `tk export --format csv` | `date,start,end,project,comment,gross,net,break` |
| `tk export --format json` | leading `[` |
| Legacy Python tool | `date,project,start_time,end_time,brutto,netto,comment` |

`gross`/`net`/`break` and `brutto`/`netto` are **derived** columns. They are parsed past and discarded; net is recomputed from the configured break tiers. Day kinds are not in any of these formats, so vacation/sick/flex days do not survive a round trip — rows landing on such a day are skipped by `ensure_work_day` and reported.

Rows that overlap an existing entry are **skipped and counted**, not fatal, so re-running an import is safe. Any other store error aborts the whole import and the transaction rolls back.

TUI flows, both from the month view, both using the existing one-field `TextPrompt`:

```
E → path box (prefilled) → write file → "Exported 143 entries to …"
I → path box → dry run → confirm showing the counts → real import → month reloads
```

## Global Constraints

- Rust edition 2024, `rust-version = "1.88"`; MSRV must not rise.
- No new dependencies. Parsing is hand-rolled, matching the hand-rolled `csv_quote`/`json_quote` on the export side.
- `./scripts/check.sh` must pass: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`.
- `E` and `I` go into `help_keys()` only, **never** into `key_hints()`. The month hint row is already at its 80-column limit (`src/tui/model.rs:1530`) and `tests/snapshots.rs::key_hints_fit_the_minimum_terminal` enforces it.
- Comments in this codebase say *why*, not *what*. Match the surrounding density.

## Review Focus

Five input classes the design implies but that no obvious happy-path test covers. Each has a test assigned to the task that owns the code.

1. **CRLF line endings** — a CSV touched by a spreadsheet on Windows must not leave `\r` glued to the last field. (Task 1)
2. **Quoted fields containing commas** — `csv_quote` emits them on export, so tk's own output must round-trip a comment like `"lunch, then review"`. (Task 1)
3. **Entries crossing midnight** — export writes `22:00,02:00` and `Entry::interval` reads `end <= start` as crossing midnight; import must accept it, not reject it as reversed. (Task 2)
4. **Empty input** — a header with no rows, and a zero-byte file, must report `imported 0` rather than erroring or panicking. (Task 3)
5. **Unreadable path** — a missing file must produce a clear error in the CLI, and a `Failed` status line in the TUI rather than a panic. (Task 5)

---

### Task 1: The parser

**Files:**
- Create: `src/core/import.rs`
- Modify: `src/core/mod.rs` (add `pub mod import;`)

**Interfaces:**
- Produces: `ImportRow { date: NaiveDate, start: NaiveTime, end: NaiveTime, project: String, comment: String, line: usize }`, `ImportError`, and `pub fn parse_import(text: &str) -> Result<Vec<ImportRow>, ImportError>`. `line` is the 1-based source line, used in skip reports.

- [ ] **Step 1: Write the failing tests**

In a new `src/core/import.rs`, tests module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn reads_the_tk_export_header() {
        let text = "date,start,end,project,comment,gross,net,break\n\
                    2026-09-14,09:00,15:30,Alpha,note,06:30,05:42,00:48\n";
        let rows = parse_import(text).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].date, d(2026, 9, 14));
        assert_eq!(rows[0].project, "Alpha");
        assert_eq!(rows[0].comment, "note");
        assert_eq!(rows[0].line, 2);
    }

    #[test]
    fn reads_the_legacy_python_header() {
        // Different column order, and the derived columns are named in German.
        let text = "date,project,start_time,end_time,brutto,netto,comment\n\
                    2026-09-14,Alpha,09:00,15:30,6.5,5.7,note\n";
        let rows = parse_import(text).unwrap();
        assert_eq!(rows[0].project, "Alpha");
        assert_eq!(rows[0].start, NaiveTime::from_hms_opt(9, 0, 0).unwrap());
        assert_eq!(rows[0].comment, "note");
    }

    #[test]
    fn reads_json() {
        let text = r#"[{"date":"2026-09-14","start":"09:00","end":"15:30","project":"Alpha","comment":"note","gross_minutes":390,"net_minutes":342,"break_minutes":48}]"#;
        let rows = parse_import(text).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].project, "Alpha");
    }

    // Review Focus 1: a spreadsheet on Windows writes CRLF.
    #[test]
    fn crlf_does_not_stick_to_the_last_field() {
        let text = "date,start,end,project,comment,gross,net,break\r\n\
                    2026-09-14,09:00,15:30,Alpha,note,06:30,05:42,00:48\r\n";
        let rows = parse_import(text).unwrap();
        assert_eq!(rows[0].comment, "note");
    }

    // Review Focus 2: csv_quote emits these on export, so they must come back.
    #[test]
    fn quoted_fields_keep_their_commas_and_quotes() {
        let text = "date,start,end,project,comment,gross,net,break\n\
                    2026-09-14,09:00,15:30,Alpha,\"lunch, then \"\"review\"\"\",06:30,05:42,00:48\n";
        let rows = parse_import(text).unwrap();
        assert_eq!(rows[0].comment, "lunch, then \"review\"");
    }

    #[test]
    fn a_bad_row_names_its_line() {
        let text = "date,start,end,project,comment,gross,net,break\n\
                    2026-09-14,not-a-time,15:30,Alpha,,,,\n";
        let err = parse_import(text).unwrap_err();
        assert!(err.to_string().contains("line 2"), "{err}");
    }

    #[test]
    fn an_unknown_header_is_refused() {
        let err = parse_import("alpha,beta\n1,2\n").unwrap_err();
        assert!(err.to_string().contains("unrecognised"), "{err}");
    }

    // Review Focus 4, parser half: no rows is not an error.
    #[test]
    fn a_header_with_no_rows_is_empty_not_an_error() {
        let text = "date,start,end,project,comment,gross,net,break\n";
        assert!(parse_import(text).unwrap().is_empty());
        assert!(parse_import("").unwrap().is_empty());
    }
}
```

- [ ] **Step 2: Run them, expect a compile error**

Run: `nix develop -c cargo test --lib core::import`
Expected: FAIL — `cannot find function 'parse_import'`.

- [ ] **Step 3: Implement the parser**

Above the tests in `src/core/import.rs`:

```rust
//! Reading entries back in: `tk export`'s own CSV and JSON, and the CSV the
//! Python tool this project replaced used to write.
//!
//! The `gross`/`net`/`break` columns (`brutto`/`netto` in the old tool) are
//! parsed past and thrown away. They are derived from the break tiers, so
//! trusting a file's copy of them would let an import contradict the config.

use chrono::{NaiveDate, NaiveTime};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportRow {
    pub date: NaiveDate,
    pub start: NaiveTime,
    pub end: NaiveTime,
    pub project: String,
    pub comment: String,
    /// 1-based line in the source file, so a skip can say which row it was.
    pub line: usize,
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("line {line}: {what}")]
    Row { line: usize, what: String },
    #[error("unrecognised header; expected a tk export or the old tool's CSV")]
    Header,
}

/// Detect the format from the text itself and parse every row.
pub fn parse_import(text: &str) -> Result<Vec<ImportRow>, ImportError> {
    if text.trim_start().starts_with('[') {
        return parse_json(text);
    }
    parse_csv(text)
}

/// Column indices for the fields we keep, resolved from the header.
struct Layout {
    date: usize,
    start: usize,
    end: usize,
    project: usize,
    comment: usize,
}

fn layout_of(header: &[String]) -> Option<Layout> {
    let at = |name: &str| header.iter().position(|h| h == name);
    // tk's own export, and the old Python tool's, differ in order and in the
    // names of the two time columns; everything else we want is common.
    let start = at("start").or_else(|| at("start_time"))?;
    let end = at("end").or_else(|| at("end_time"))?;
    Some(Layout {
        date: at("date")?,
        start,
        end,
        project: at("project")?,
        comment: at("comment")?,
    })
}

fn parse_csv(text: &str) -> Result<Vec<ImportRow>, ImportError> {
    let mut lines = text.lines().enumerate();
    let Some((_, header_line)) = lines.next() else {
        return Ok(Vec::new());
    };
    if header_line.trim().is_empty() {
        return Ok(Vec::new());
    }
    let layout = layout_of(&split_csv(header_line)).ok_or(ImportError::Header)?;
    let mut rows = Vec::new();
    for (i, line) in lines {
        let line_no = i + 1;
        if line.trim().is_empty() {
            continue;
        }
        let f = split_csv(line);
        let get = |idx: usize| f.get(idx).map(String::as_str).unwrap_or_default();
        rows.push(row_from(
            line_no,
            get(layout.date),
            get(layout.start),
            get(layout.end),
            get(layout.project),
            get(layout.comment),
        )?);
    }
    Ok(rows)
}

fn row_from(
    line: usize,
    date: &str,
    start: &str,
    end: &str,
    project: &str,
    comment: &str,
) -> Result<ImportRow, ImportError> {
    let bad = |what: String| ImportError::Row { line, what };
    let time = |s: &str| {
        NaiveTime::parse_from_str(s, "%H:%M")
            .map_err(|_| bad(format!("'{s}' is not a HH:MM time")))
    };
    if project.trim().is_empty() {
        return Err(bad("no project".into()));
    }
    Ok(ImportRow {
        date: NaiveDate::parse_from_str(date, "%Y-%m-%d")
            .map_err(|_| bad(format!("'{date}' is not a YYYY-MM-DD date")))?,
        start: time(start)?,
        end: time(end)?,
        project: project.trim().to_string(),
        comment: comment.trim().to_string(),
        line,
    })
}

/// One CSV line into fields, honouring the `""`-escaped quoting `csv_quote`
/// writes. A trailing `\r` is a line ending, not data.
fn split_csv(line: &str) -> Vec<String> {
    let line = line.strip_suffix('\r').unwrap_or(line);
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if in_quotes && chars.peek() == Some(&'"') => {
                cur.push('"');
                chars.next();
            }
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => out.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    out.push(cur);
    out
}

/// The flat array `tk export --format json` writes. Hand-rolled to match the
/// hand-rolled writer; the shape is fixed and shallow.
fn parse_json(text: &str) -> Result<Vec<ImportRow>, ImportError> {
    let mut rows = Vec::new();
    for (i, obj) in text
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split("},")
        .enumerate()
    {
        let obj = obj.trim().trim_start_matches('{').trim_end_matches('}');
        if obj.trim().is_empty() {
            continue;
        }
        let line = i + 1;
        let field = |name: &str| -> String {
            let needle = format!("\"{name}\":\"");
            match obj.find(&needle) {
                Some(p) => {
                    let rest = &obj[p + needle.len()..];
                    let end = rest.find('"').unwrap_or(rest.len());
                    rest[..end].replace("\\\"", "\"").replace("\\\\", "\\")
                }
                None => String::new(),
            }
        };
        rows.push(row_from(
            line,
            &field("date"),
            &field("start"),
            &field("end"),
            &field("project"),
            &field("comment"),
        )?);
    }
    Ok(rows)
}
```

Add to `src/core/mod.rs`, in the existing `pub mod` list:

```rust
pub mod import;
```

- [ ] **Step 4: Run the tests**

Run: `nix develop -c cargo test --lib core::import`
Expected: PASS, all eight.

- [ ] **Step 5: Commit**

```bash
git add src/core/import.rs src/core/mod.rs
git commit -m "feat(core): parse tk's own export and the old tool's CSV back into rows"
```

---

### Task 2: `Store::import_entries`

**Files:**
- Modify: `src/store/entries.rs` (the implementation), `src/store/mod.rs` (the tests)

**Interfaces:**
- Consumes: `crate::core::import::ImportRow` from Task 1.
- Produces: `ImportReport { imported: usize, skipped: Vec<(usize, String)>, new_projects: Vec<String> }` and `pub fn import_entries(&self, rows: &[ImportRow], dry_run: bool) -> StoreResult<ImportReport>`.

- [ ] **Step 1: Write the failing tests**

All store tests live in the `mod tests` of **`src/store/mod.rs`** (`src/store/entries.rs` has none), which already provides `d(y, m, dd)`, `t(h, m)` and imports `CoreError`/`DayKind`. Add there, opening the store the way the existing tests do:

```rust
#[test]
fn imports_rows_and_creates_their_projects() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(&dir.path().join("tk.db")).unwrap();
    let rows = vec![
        import_row(1, d(2026, 9, 14), (9, 0), (15, 30), "Alpha"),
        import_row(2, d(2026, 9, 15), (9, 0), (17, 0), "Beta"),
    ];
    let r = s.import_entries(&rows, false).unwrap();
    assert_eq!(r.imported, 2);
    assert!(r.skipped.is_empty());
    assert_eq!(r.new_projects, vec!["Alpha".to_string(), "Beta".to_string()]);
    assert_eq!(s.entries_in(d(2026, 9, 14), d(2026, 9, 15)).unwrap().len(), 2);
}

#[test]
fn an_overlapping_row_is_skipped_not_fatal() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(&dir.path().join("tk.db")).unwrap();
    let rows = vec![import_row(1, d(2026, 9, 14), (9, 0), (15, 30), "Alpha")];
    s.import_entries(&rows, false).unwrap();

    // Same file again: the row collides with what the first run wrote.
    let r = s.import_entries(&rows, false).unwrap();
    assert_eq!(r.imported, 0);
    assert_eq!(r.skipped.len(), 1);
    assert_eq!(r.skipped[0].0, 1);
    // CoreError::Overlap's message, carried through as the skip reason.
    assert!(r.skipped[0].1.contains("overlaps"), "{:?}", r.skipped);
    assert_eq!(s.entries_in(d(2026, 9, 14), d(2026, 9, 14)).unwrap().len(), 1);
}

#[test]
fn dry_run_reports_but_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(&dir.path().join("tk.db")).unwrap();
    let rows = vec![import_row(1, d(2026, 9, 14), (9, 0), (15, 30), "Alpha")];
    let r = s.import_entries(&rows, true).unwrap();
    assert_eq!(r.imported, 1);
    assert!(s.entries_in(d(2026, 9, 14), d(2026, 9, 14)).unwrap().is_empty());
    assert!(s.project_by_name("Alpha").unwrap().is_none());
}

#[test]
fn a_row_on_a_non_work_day_is_skipped_with_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(&dir.path().join("tk.db")).unwrap();
    s.set_day_kind(d(2026, 9, 14), &DayKind::Vacation).unwrap();
    let rows = vec![import_row(1, d(2026, 9, 14), (9, 0), (15, 30), "Alpha")];
    let r = s.import_entries(&rows, false).unwrap();
    assert_eq!(r.imported, 0);
    assert!(r.skipped[0].1.contains("vacation"), "{:?}", r.skipped);
}

// Review Focus 3: export writes 22:00,02:00 for a shift over midnight.
#[test]
fn an_entry_crossing_midnight_imports() {
    let dir = tempfile::tempdir().unwrap();
    let s = Store::open(&dir.path().join("tk.db")).unwrap();
    let rows = vec![import_row(1, d(2026, 9, 14), (22, 0), (2, 0), "Alpha")];
    let r = s.import_entries(&rows, false).unwrap();
    assert_eq!(r.imported, 1, "{:?}", r.skipped);
    let e = &s.entries_in(d(2026, 9, 14), d(2026, 9, 14)).unwrap()[0];
    assert!(e.crosses_midnight());
}
```

And the helper, next to the other test helpers in that module:

```rust
fn import_row(
    line: usize,
    date: NaiveDate,
    start: (u32, u32),
    end: (u32, u32),
    project: &str,
) -> crate::core::import::ImportRow {
    crate::core::import::ImportRow {
        date,
        start: NaiveTime::from_hms_opt(start.0, start.1, 0).unwrap(),
        end: NaiveTime::from_hms_opt(end.0, end.1, 0).unwrap(),
        project: project.to_string(),
        comment: String::new(),
        line,
    }
}
```

- [ ] **Step 2: Run them, expect a compile error**

Run: `nix develop -c cargo test --lib store::entries`
Expected: FAIL — `no method named 'import_entries'`.

- [ ] **Step 3: Implement it**

In `src/store/entries.rs`, next to `add_entry`:

```rust
/// What an import did, for the message the CLI prints and the TUI shows.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub imported: usize,
    /// (source line, why), one per row that could not be written.
    pub skipped: Vec<(usize, String)>,
    pub new_projects: Vec<String>,
}

impl Store {
    /// Apply `rows` in a single transaction.
    ///
    /// A row that breaks a rule the store already enforces — an overlap, a day
    /// that is not a work day — is skipped and counted rather than aborting the
    /// file, so re-running an import lands only what is genuinely new. Anything
    /// else is a real failure and rolls the whole import back.
    ///
    /// `dry_run` does every check and then rolls back, which is how the caller
    /// can show what *would* happen without a second code path that might
    /// disagree with the real one.
    pub fn import_entries(&self, rows: &[ImportRow], dry_run: bool) -> StoreResult<ImportReport> {
        self.conn().execute_batch("BEGIN IMMEDIATE")?;
        let result = self.import_locked(rows);
        if result.is_ok() && !dry_run {
            self.conn().execute_batch("COMMIT")?;
        } else {
            let _ = self.conn().execute_batch("ROLLBACK");
        }
        result
    }

    fn import_locked(&self, rows: &[ImportRow]) -> StoreResult<ImportReport> {
        let mut report = ImportReport::default();
        for r in rows {
            // Asked before the write, because the write is what creates it.
            let is_new = self.project_by_name(r.project.trim())?.is_none()
                && !report.new_projects.iter().any(|p| p == r.project.trim());
            match self.add_entry_locked(r.date, r.start, r.end, &r.project, &r.comment) {
                Ok(_) => {
                    report.imported += 1;
                    if is_new {
                        report.new_projects.push(r.project.trim().to_string());
                    }
                }
                // Exactly the three a bad row can legitimately trip:
                // `ensure_work_day` raises Constraint, `check_overlap` raises
                // CoreError::Overlap and `check_range` CoreError::InvalidRange.
                // Anything else is the database in trouble, not this row.
                Err(StoreError::Constraint(why)) => report.skipped.push((r.line, why)),
                Err(StoreError::Core(e @ (CoreError::Overlap | CoreError::InvalidRange))) => {
                    report.skipped.push((r.line, e.to_string()))
                }
                Err(e) => return Err(e),
            }
        }
        Ok(report)
    }
}
```

Add to the file's imports: `use crate::core::import::ImportRow;` and `use crate::core::CoreError;` (that is the re-exported path `src/store/mod.rs` already uses).

- [ ] **Step 4: Run the tests**

Run: `nix develop -c cargo test --lib store::entries`
Expected: PASS, all five new ones plus the existing ones.

- [ ] **Step 5: Commit**

```bash
git add src/store/entries.rs
git commit -m "feat(store): import rows in one transaction, skipping what collides"
```

---

### Task 3: `tk import`

**Files:**
- Modify: `src/cli/mod.rs` (the `Command` enum), `src/cli/commands.rs` (the match arm), `README.md`
- Test: `tests/cli.rs`

**Interfaces:**
- Consumes: `parse_import` (Task 1), `Store::import_entries` and `ImportReport` (Task 2).
- Produces: `pub fn import_file(ctx: &Ctx, path: &Path, dry_run: bool) -> anyhow::Result<(ImportReport, String)>` in `src/cli/commands.rs` — the summary `String` is the one-line message, reused verbatim by the TUI in Task 5.

- [ ] **Step 1: Write the failing tests**

In `tests/cli.rs`:

```rust
#[test]
fn export_then_import_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let csv = dir.path().join("out.csv");
    tk(home)
        .args(["add", "2026-09-14", "0900-1530", "-p", "Alpha", "-m", "lunch, then review"])
        .assert()
        .success();
    tk(home).args(["export", "-o", csv.to_str().unwrap()]).assert().success();

    // A fresh home: the same rows must land from the file alone.
    let home2 = dir.path().join("h2");
    tk(&home2)
        .args(["import", csv.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("imported 1"));
    tk(&home2)
        .args(["export"])
        .assert()
        .success()
        .stdout(predicate::str::contains("lunch, then review"));
}

#[test]
fn importing_the_same_file_twice_skips_everything() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let csv = dir.path().join("out.csv");
    std::fs::write(
        &csv,
        "date,start,end,project,comment,gross,net,break\n\
         2026-09-14,09:00,15:30,Alpha,note,06:30,05:42,00:48\n",
    )
    .unwrap();
    tk(home).args(["import", csv.to_str().unwrap()]).assert().success();
    tk(home)
        .args(["import", csv.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("skipped 1"));
}

#[test]
fn dry_run_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path();
    let csv = dir.path().join("out.csv");
    std::fs::write(
        &csv,
        "date,start,end,project,comment,gross,net,break\n\
         2026-09-14,09:00,15:30,Alpha,note,06:30,05:42,00:48\n",
    )
    .unwrap();
    tk(home)
        .args(["import", csv.to_str().unwrap(), "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("imported 1"));
    tk(home)
        .args(["export"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Alpha").not());
}

// Review Focus 4: nothing to do is not a failure.
#[test]
fn an_empty_file_imports_nothing_without_erroring() {
    let dir = tempfile::tempdir().unwrap();
    let csv = dir.path().join("empty.csv");
    std::fs::write(&csv, "").unwrap();
    tk(dir.path())
        .args(["import", csv.to_str().unwrap()])
        .assert()
        .success()
        .stdout(predicate::str::contains("imported 0"));
}

// Review Focus 5, CLI half.
#[test]
fn a_missing_file_fails_with_its_path() {
    let dir = tempfile::tempdir().unwrap();
    tk(dir.path())
        .args(["import", "/nope/missing.csv"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("missing.csv"));
}
```

- [ ] **Step 2: Run them, expect a failure**

Run: `nix develop -c cargo test --test cli`
Expected: FAIL — clap rejects the unknown subcommand `import`.

- [ ] **Step 3: Add the subcommand**

In `src/cli/mod.rs`, in `enum Command`, after `Export`:

```rust
    /// Import entries from a tk export (CSV or JSON) or the old tool's CSV
    Import {
        /// File to read
        file: PathBuf,
        /// Report what would happen and write nothing
        #[arg(long)]
        dry_run: bool,
    },
```

In `src/cli/commands.rs`, the shared worker plus the arm:

```rust
/// Read `path`, apply it, and return the report with the one-line summary that
/// both the CLI and the TUI show.
pub fn import_file(
    ctx: &Ctx,
    path: &Path,
    dry_run: bool,
) -> anyhow::Result<(ImportReport, String)> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let rows = parse_import(&text)?;
    let report = ctx.store.import_entries(&rows, dry_run)?;
    let mut msg = format!("imported {}", report.imported);
    if !report.skipped.is_empty() {
        msg.push_str(&format!(", skipped {}", report.skipped.len()));
    }
    if !report.new_projects.is_empty() {
        msg.push_str(&format!(", {} new projects", report.new_projects.len()));
    }
    if dry_run {
        msg.push_str(" (dry run, nothing written)");
    }
    Ok((report, msg))
}
```

```rust
        Command::Import { file, dry_run } => {
            let (report, msg) = import_file(ctx, &file, dry_run)?;
            writeln!(out, "{msg}")?;
            // The lines say which rows need a look; the count alone does not.
            for (line, why) in &report.skipped {
                writeln!(out, "  line {line}: {why}")?;
            }
        }
```

Add `use anyhow::Context;`, `use std::path::Path;`, `use crate::core::import::parse_import;` and `use crate::store::entries::ImportReport;` to the file's imports if they are not already there.

- [ ] **Step 4: Run the tests**

Run: `nix develop -c cargo test --test cli`
Expected: PASS.

- [ ] **Step 5: README**

In the "Command line" section, after the `export` entry, matching the surrounding table/prose style:

```markdown
### `tk import FILE [--dry-run]`

Reads entries back in from a `tk export` (CSV or JSON) or from the CSV the
Python tool this project replaced used to write. The format is detected from
the file; there is no flag for it.

The `gross`, `net` and `break` columns are ignored and recomputed from your
break tiers, so an import can never contradict your configuration. Projects
that do not exist yet are created. Rows that overlap an entry you already have,
or that land on a day marked vacation, flex, sick or holiday, are skipped and
listed by line number — so running the same import twice is safe.

`--dry-run` prints exactly the same report and writes nothing.

Day types are not part of any of these formats and are not restored.
```

- [ ] **Step 6: Commit**

```bash
git add src/cli/mod.rs src/cli/commands.rs tests/cli.rs README.md
git commit -m "feat(cli): tk import, with --dry-run and a per-line skip report"
```

---

### Task 4: `E` exports from the TUI

**Files:**
- Modify: `src/tui/msg.rs`, `src/tui/model.rs`, `src/tui/worker.rs`, `src/tui/components/month.rs`, `README.md`

**Interfaces:**
- Consumes: the existing `open_prompt`/`PromptKind` machinery (`src/tui/model.rs:659`).
- Produces: `PromptKind::ExportPath`, `Msg::ExportPrompt`, `StoreCmd::Export { path: PathBuf }`, and an `open_prompt` that takes a field label. Task 5 reuses all four.

- [ ] **Step 1: Write the failing tests**

In `src/tui/components/month.rs` tests:

```rust
#[test]
fn shift_e_asks_where_to_export() {
    let mut c = MonthScreen::default();
    let ev = Event::Keyboard(KeyEvent::new(Key::Char('E'), KeyModifiers::SHIFT));
    assert_eq!(c.on(&ev), Some(Msg::ExportPrompt));
}
```

In `src/tui/model.rs` tests, beside the existing prompt tests:

```rust
#[test]
fn the_export_prompt_opens_prefilled_and_submits_a_path() {
    let (mut m, mut rx) = model(d(2026, 9, 25));
    m.update(Msg::ExportPrompt);
    assert!(matches!(m.prompt, Some(PromptKind::ExportPath)));
    m.update(Msg::PromptSubmit("/tmp/out.csv".into()));
    assert!(m.prompt.is_none());
    match rx.try_recv().unwrap() {
        StoreCmd::Export { path } => assert_eq!(path, std::path::PathBuf::from("/tmp/out.csv")),
        other => panic!("expected Export, got {other:?}"),
    }
}
```

In `src/tui/worker.rs` tests, following `StoreCmd::Backup`'s test:

```rust
#[test]
fn export_writes_the_file_and_says_so() {
    let home = tempfile::tempdir().unwrap();
    let ctx = ctx(home.path());
    let out = home.path().join("out.csv");
    let reply = handle(&ctx, StoreCmd::Export { path: out.clone() }).unwrap();
    let StoreReply::Changed(msg) = reply else {
        panic!("expected Changed, got {reply:?}")
    };
    assert!(msg.starts_with("Exported to "), "{msg}");
    assert!(out.exists());
}
```

- [ ] **Step 2: Run them, expect compile errors**

Run: `nix develop -c cargo test --lib tui`
Expected: FAIL — `no variant named ExportPrompt`.

- [ ] **Step 3: Implement**

`src/tui/msg.rs` — in `Msg`, beside the `Backup` variant:

```rust
    /// `E` on the month screen: ask where to write the export.
    ExportPrompt,
```

in `PromptKind` (`src/tui/model.rs:94`):

```rust
    ExportPath,
```

and in `StoreCmd`:

```rust
    Export {
        path: std::path::PathBuf,
    },
```

`src/tui/components/month.rs`, next to `Key::Char('B')`:

```rust
            Key::Char('E') => Msg::ExportPrompt,
```

`src/tui/model.rs` — give `open_prompt` a label, since the box is no longer only for names:

```rust
    fn open_prompt(&mut self, title: String, label: &str, initial: &str, kind: PromptKind) {
        let _ = self.app.umount(&Id::Prompt);
        let _ = self.app.mount(
            Id::Prompt,
            Box::new(
                components::text_prompt::TextPrompt::new(title, label, initial)
                    .with_theme(&self.theme),
            ),
            vec![],
        );
        self.prompt = Some(kind);
        self.focus(Id::Prompt);
    }
```

Update the two existing callers (`ProjectsRename`, `ProjectsAdd`) to pass `"Name"`. Then the new arm:

```rust
            Msg::ExportPrompt => {
                // Into the data directory, where backups already go: always
                // writable, and the user knows where it is.
                let default = self
                    .home
                    .join(format!("export-{}.csv", self.today))
                    .display()
                    .to_string();
                self.open_prompt("Export".into(), "Path", &default, PromptKind::ExportPath);
            }
```

and in the `Msg::PromptSubmit(text)` arm, alongside the project cases:

```rust
                    PromptKind::ExportPath => {
                        self.send(StoreCmd::Export { path: text.trim().into() })
                    }
```

`src/tui/worker.rs`, beside the `Backup` arm:

```rust
        StoreCmd::Export { path } => {
            crate::cli::commands::write_export(ctx, &path)?;
            StoreReply::Changed(format!("Exported to {}", path.display()))
        }
```

This needs the export body lifted out of the `Command::Export` arm in `src/cli/commands.rs` into `pub fn write_export(ctx: &Ctx, path: &Path) -> anyhow::Result<()>`, with the arm calling it — the same extraction `write_backup` already had. Format follows the extension: `.json` gives JSON, anything else CSV.

Add to `help_keys()` for `Screen::Month`, after the `B` line:

```rust
                ("E", "export to a file"),
```

Do **not** touch `key_hints()`.

- [ ] **Step 4: Run the tests**

Run: `nix develop -c cargo test --lib tui` then `nix develop -c cargo test --test snapshots`
Expected: PASS both — the snapshot suite proves the hint row is untouched.

- [ ] **Step 5: README**

In "Month view", in the key table after `B`:

```markdown
| `E` | Export to a file (asks for the path, `.json` for JSON) |
```

- [ ] **Step 6: Commit**

```bash
git add src/tui src/cli/commands.rs README.md
git commit -m "feat(tui): E exports from the month view"
```

---

### Task 5: `I` imports from the TUI

**Files:**
- Modify: `src/tui/msg.rs`, `src/tui/model.rs`, `src/tui/worker.rs`, `src/tui/components/month.rs`, `README.md`

**Interfaces:**
- Consumes: `import_file` (Task 3), `open_prompt`'s label parameter and `PromptKind` (Task 4).
- Produces: `PromptKind::ImportPath`, `Msg::ImportPrompt`, `StoreCmd::ImportDryRun { path }`, `StoreCmd::Import { path }`, `Confirm::ImportFile { path, imported, skipped }`.

- [ ] **Step 1: Write the failing tests**

`src/tui/components/month.rs`:

```rust
#[test]
fn shift_i_asks_which_file_to_import() {
    let mut c = MonthScreen::default();
    let ev = Event::Keyboard(KeyEvent::new(Key::Char('I'), KeyModifiers::SHIFT));
    assert_eq!(c.on(&ev), Some(Msg::ImportPrompt));
}

#[test]
fn lowercase_i_still_opens_the_clock_picker() {
    let mut c = MonthScreen::default();
    let ev = Event::Keyboard(KeyEvent::new(Key::Char('i'), KeyModifiers::NONE));
    assert_eq!(c.on(&ev), Some(Msg::OpenClockPicker));
}
```

`src/tui/model.rs`:

```rust
#[test]
fn import_dry_runs_then_confirms_then_writes() {
    let (mut m, mut rx) = model(d(2026, 9, 25));
    m.update(Msg::ImportPrompt);
    assert!(matches!(m.prompt, Some(PromptKind::ImportPath)));

    m.update(Msg::PromptSubmit("/tmp/in.csv".into()));
    match rx.try_recv().unwrap() {
        StoreCmd::ImportDryRun { path } => {
            assert_eq!(path, std::path::PathBuf::from("/tmp/in.csv"))
        }
        other => panic!("expected ImportDryRun, got {other:?}"),
    }

    // The dry run's counts come back as the confirm dialog.
    m.update(Msg::AskConfirm(Confirm::ImportFile {
        path: "/tmp/in.csv".into(),
        imported: 143,
        skipped: 12,
    }));
    let text = m.confirm_text(m.confirm.as_ref().unwrap());
    assert!(text.contains("143"), "{text}");
    assert!(text.contains("12"), "{text}");

    m.update(Msg::ConfirmYes);
    match rx.try_recv().unwrap() {
        StoreCmd::Import { path } => assert_eq!(path, std::path::PathBuf::from("/tmp/in.csv")),
        other => panic!("expected Import, got {other:?}"),
    }
}

#[test]
fn declining_the_import_confirm_writes_nothing() {
    let (mut m, mut rx) = model(d(2026, 9, 25));
    m.update(Msg::AskConfirm(Confirm::ImportFile {
        path: "/tmp/in.csv".into(),
        imported: 1,
        skipped: 0,
    }));
    m.update(Msg::ConfirmNo);
    assert!(rx.try_recv().is_err(), "nothing should have been sent");
}
```

`src/tui/worker.rs`:

```rust
#[test]
fn import_dry_run_reports_without_writing() {
    let home = tempfile::tempdir().unwrap();
    let ctx = ctx(home.path());
    let csv = home.path().join("in.csv");
    std::fs::write(
        &csv,
        "date,start,end,project,comment,gross,net,break\n\
         2026-09-14,09:00,15:30,Alpha,note,06:30,05:42,00:48\n",
    )
    .unwrap();
    let reply = handle(&ctx, StoreCmd::ImportDryRun { path: csv.clone() }).unwrap();
    assert!(matches!(
        reply,
        StoreReply::Changed(_) | StoreReply::Failed(_)
    ));
    assert!(ctx.store.project_by_name("Alpha").unwrap().is_none());
}

// Review Focus 5, TUI half: a bad path is a status line, never a panic.
#[test]
fn importing_a_missing_file_replies_failed() {
    let home = tempfile::tempdir().unwrap();
    let ctx = ctx(home.path());
    let reply = handle(&ctx, StoreCmd::Import { path: "/nope/missing.csv".into() });
    assert!(reply.is_err() || matches!(reply, Ok(StoreReply::Failed(_))));
}
```

- [ ] **Step 2: Run them, expect compile errors**

Run: `nix develop -c cargo test --lib tui`
Expected: FAIL — `no variant named ImportPrompt`.

- [ ] **Step 3: Implement**

`src/tui/msg.rs` — `Msg`:

```rust
    /// `I` on the month screen: ask which file to import.
    ImportPrompt,
```

`Confirm`:

```rust
    /// The dry run's counts, shown before anything is written.
    ImportFile {
        path: std::path::PathBuf,
        imported: usize,
        skipped: usize,
    },
```

`StoreCmd`:

```rust
    ImportDryRun {
        path: std::path::PathBuf,
    },
    Import {
        path: std::path::PathBuf,
    },
```

`PromptKind` in `src/tui/model.rs`:

```rust
    ImportPath,
```

`src/tui/components/month.rs`, next to `Key::Char('E')`:

```rust
            Key::Char('I') => Msg::ImportPrompt,
```

`src/tui/model.rs` — the arms:

```rust
            Msg::ImportPrompt => {
                self.open_prompt("Import".into(), "Path", "", PromptKind::ImportPath)
            }
```

in `Msg::PromptSubmit`:

```rust
                    PromptKind::ImportPath => {
                        self.send(StoreCmd::ImportDryRun { path: text.trim().into() })
                    }
```

in `confirm_text`:

```rust
            Confirm::ImportFile {
                path,
                imported,
                skipped,
            } => {
                let mut s = format!("Import {imported} entries from {}?", path.display());
                if *skipped > 0 {
                    // Named here because this dialog is the only place the user
                    // sees them before the write happens.
                    s.push_str(&format!(
                        " {skipped} rows overlap existing entries and will be skipped."
                    ));
                }
                s
            }
```

and in the `Msg::ConfirmYes` match over the pending confirm:

```rust
            Confirm::ImportFile { path, .. } => self.send(StoreCmd::Import { path: path.clone() }),
```

`src/tui/worker.rs` — both arms, reusing Task 3's shared function so the dry run and the real run cannot drift:

```rust
        StoreCmd::ImportDryRun { path } => {
            let (report, _) = crate::cli::commands::import_file(ctx, &path, true)?;
            StoreReply::Confirm(Confirm::ImportFile {
                path,
                imported: report.imported,
                skipped: report.skipped.len(),
            })
        }
        StoreCmd::Import { path } => {
            let (_, msg) = crate::cli::commands::import_file(ctx, &path, false)?;
            StoreReply::Changed(msg)
        }
```

This needs one new `StoreReply` variant in `src/tui/msg.rs`:

```rust
    /// A dry run's result, asked about before anything is written.
    Confirm(Confirm),
```

and its arm in `on_store` (`src/tui/model.rs:1307`), beside `StoreReply::Changed`:

```rust
            StoreReply::Confirm(c) => self.open_confirm(c),
```

`open_confirm` is what `Msg::AskConfirm` already calls (`model.rs:925`), so the dialog, `ConfirmYes` and `ConfirmNo` all work unchanged.

Nothing extra is needed to refresh the month: `StoreReply::Changed` already ends in `self.reload_after_write()` (`model.rs:1337`), and the real import returns `Changed`.

Add to `help_keys()` for `Screen::Month`, after the `E` line:

```rust
                ("I", "import a file (asks first)"),
```

Again, do **not** touch `key_hints()`.

- [ ] **Step 4: Run everything**

Run: `nix develop -c cargo test`
Expected: PASS, whole suite.

- [ ] **Step 5: README**

In "Month view", after the `E` row:

```markdown
| `I` | Import a file (shows what it would do, then asks) |
```

And in "Overlays":

```markdown
| Import confirm (`I`) | Shows the dry run's counts; `y` or `Enter` imports, `n` or `Esc` cancels |
```

- [ ] **Step 6: Full check and commit**

```bash
./scripts/check.sh
git add src/tui README.md
git commit -m "feat(tui): I imports a file, after showing what the dry run found"
```

---

## Done when

`./scripts/check.sh` is green, `tk import` round-trips its own export, and `E` / `I` work from the month view and are listed in `?` but not in the hint row.
