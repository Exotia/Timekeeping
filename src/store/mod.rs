//! SQLite-backed store.

mod days;
mod entries;
mod projects;
mod session;

use std::path::Path;

use rusqlite::Connection;
use thiserror::Error;

use crate::core::CoreError;

pub use session::{Session, SessionState};

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
        // An existing database states its version before anything is written to it, so a
        // schema from a newer `tk` is refused with its tables still intact rather than
        // having our DDL applied on top of it.
        let has_meta: i64 = conn.query_row(
            "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name = 'meta'",
            [],
            |r| r.get(0),
        )?;
        if has_meta > 0 {
            migrate(&conn, schema_version_of(&conn)?)?;
            check_version(schema_version_of(&conn)?)?;
        }
        // One transaction: a failure part-way through leaves no half-created schema.
        // Every statement is `IF NOT EXISTS`, so on a database that already exists this
        // only adds what a migration has not: a fresh file gets the whole current schema
        // and states version 2 straight away.
        conn.execute_batch(&format!("BEGIN;\n{SCHEMA}\nCOMMIT;"))?;
        let store = Store { conn };
        check_version(store.schema_version()?)?;
        Ok(store)
    }

    pub fn schema_version(&self) -> StoreResult<i64> {
        schema_version_of(&self.conn)
    }

    pub fn backup_to(&self, path: &Path) -> StoreResult<()> {
        let p = path.to_string_lossy().to_string();
        self.conn.execute("VACUUM INTO ?1", [p])?;
        Ok(())
    }

    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    /// Runs `f` inside an immediate write transaction, so that a check
    /// (e.g. overlap detection, entry-count check) and its dependent write
    /// are atomic against other writers on the same database file. `f` must
    /// not itself start a transaction.
    pub(crate) fn in_write_tx<T>(&self, f: impl FnOnce() -> StoreResult<T>) -> StoreResult<T> {
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        match f() {
            Ok(v) => {
                self.conn.execute_batch("COMMIT")?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }
}

/// The only schema version this build understands.
const SCHEMA_VERSION: i64 = 2;

/// Bring an existing database up to [`SCHEMA_VERSION`], one numbered step at a
/// time, each in its own transaction so a failure leaves the database on the
/// version it still is. A version this build does not know is left alone here
/// and refused by [`check_version`] afterwards, with every row intact.
fn migrate(conn: &Connection, from: i64) -> StoreResult<()> {
    if from == 1 {
        // v1 → v2: explicit break shares per entry, and a session that can be
        // paused. Both are added to tables that exist, so the rows are kept.
        conn.execute_batch(
            "BEGIN;
             ALTER TABLE entries ADD COLUMN break_share INTEGER;
             ALTER TABLE session ADD COLUMN state TEXT NOT NULL DEFAULT 'working';
             UPDATE meta SET value = '2' WHERE key = 'schema_version';
             COMMIT;",
        )?;
    }
    Ok(())
}

fn check_version(v: i64) -> StoreResult<()> {
    if v == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(StoreError::Migration(format!(
            "unsupported schema version {v}"
        )))
    }
}

fn schema_version_of(conn: &Connection) -> StoreResult<i64> {
    let v: String = conn.query_row(
        "SELECT value FROM meta WHERE key = 'schema_version'",
        [],
        |r| r.get(0),
    )?;
    v.parse()
        .map_err(|_| StoreError::Migration(format!("bad schema_version {v}")))
}

pub(crate) fn date_str(d: chrono::NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

pub(crate) fn parse_date(s: &str) -> rusqlite::Result<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

/// A stored minute-of-day back into a `NaiveTime`. A row outside 0..1440 is a corrupt
/// database, not a panic: it surfaces as a conversion failure on that query.
pub(crate) fn time_from_min(m: i64) -> rusqlite::Result<chrono::NaiveTime> {
    if !(0..1440).contains(&m) {
        return Err(bad_minute(m));
    }
    chrono::NaiveTime::from_hms_opt((m / 60) as u32, (m % 60) as u32, 0)
        .ok_or_else(|| bad_minute(m))
}

fn bad_minute(m: i64) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Integer,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid stored minute-of-day {m}"),
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{CoreError, DayKind, HolidayCalendar, Minutes};
    use chrono::{NaiveDate, NaiveTime};

    fn d(y: i32, m: u32, dd: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, dd).unwrap()
    }
    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }
    /// The wall clock a clock-out is asked at: the session's own day unless said otherwise.
    fn at(date: NaiveDate, h: u32, m: u32) -> chrono::NaiveDateTime {
        chrono::NaiveDateTime::new(date, t(h, m))
    }

    #[test]
    fn opens_and_migrates_file_db() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("tk.db");
        let s = Store::open(&path).unwrap();
        assert_eq!(s.schema_version().unwrap(), 2);
        drop(s);
        let s = Store::open(&path).unwrap(); // idempotent
        assert_eq!(s.schema_version().unwrap(), 2);
    }

    #[test]
    fn a_newer_schema_is_refused_without_touching_the_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tk.db");
        // A database written by a future `tk`: it has `meta`, but nothing else we know.
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(
                "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                 INSERT INTO meta (key, value) VALUES ('schema_version', '3');
                 CREATE TABLE future_stuff (id INTEGER PRIMARY KEY);",
            )
            .unwrap();
        }
        let err = Store::open(&path)
            .err()
            .expect("a newer schema must not open");
        match err {
            StoreError::Migration(m) => assert!(m.contains('3'), "{m}"),
            other => panic!("expected a migration error, got {other:?}"),
        }
        // None of our DDL ran, and the version it states is untouched.
        let c = Connection::open(&path).unwrap();
        let tables: Vec<String> = c
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(tables, vec!["future_stuff".to_string(), "meta".into()]);
        let v: String = c
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(v, "3");
    }

    /// The v1 schema, exactly as `tk` wrote it before break shares and the
    /// break state existed: the shape of every database already in use.
    const V1_SCHEMA: &str = "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE projects (id INTEGER PRIMARY KEY, name TEXT UNIQUE NOT NULL,
             color_index INTEGER NOT NULL, archived INTEGER NOT NULL DEFAULT 0);
         CREATE TABLE days (date TEXT PRIMARY KEY, kind TEXT NOT NULL, label TEXT);
         CREATE TABLE entries (id INTEGER PRIMARY KEY, date TEXT NOT NULL,
             start_min INTEGER NOT NULL, end_min INTEGER NOT NULL,
             project_id INTEGER NOT NULL REFERENCES projects(id),
             comment TEXT NOT NULL DEFAULT '');
         CREATE INDEX entries_date ON entries(date);
         CREATE TABLE session (id INTEGER PRIMARY KEY CHECK (id = 1),
             date TEXT NOT NULL, start_min INTEGER NOT NULL, project_id INTEGER);
         INSERT INTO meta (key, value) VALUES ('schema_version', '1');
         INSERT INTO projects (id, name, color_index) VALUES (1, 'Alpha', 0);
         INSERT INTO days (date, kind, label) VALUES ('2026-09-16', 'vacation', NULL);
         INSERT INTO entries (id, date, start_min, end_min, project_id, comment)
             VALUES (7, '2026-09-15', 540, 1020, 1, 'kept');
         INSERT INTO session (id, date, start_min, project_id)
             VALUES (1, '2026-09-17', 480, 1);";

    #[test]
    fn a_v1_database_is_migrated_to_v2_and_keeps_its_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tk.db");
        {
            let c = Connection::open(&path).unwrap();
            c.execute_batch(V1_SCHEMA).unwrap();
        }
        let s = Store::open(&path).unwrap();
        assert_eq!(s.schema_version().unwrap(), 2);
        // Every v1 row is still there, and the new column reads as unassigned.
        let e = s.entry(7).unwrap();
        assert_eq!(
            (e.date, e.start, e.end),
            (d(2026, 9, 15), t(9, 0), t(17, 0))
        );
        assert_eq!((e.project.as_str(), e.comment.as_str()), ("Alpha", "kept"));
        assert_eq!(e.break_share, None);
        assert_eq!(
            s.stored_kind(d(2026, 9, 16)).unwrap(),
            Some(DayKind::Vacation)
        );
        assert_eq!(s.list_projects(true).unwrap().len(), 1);
        // The session it was left clocked in on survives, working as before.
        let sess = s.session().unwrap().expect("the open session is kept");
        assert_eq!((sess.date, sess.start), (d(2026, 9, 17), t(8, 0)));
        assert_eq!(sess.state, SessionState::Working);
        // And the new columns are usable straight away.
        s.set_break_shares(&[(7, Some(crate::core::Minutes(20)))])
            .unwrap();
        assert_eq!(
            s.entry(7).unwrap().break_share,
            Some(crate::core::Minutes(20))
        );
        drop(s);
        // Opening it again is a no-op: the migration does not run twice.
        let s = Store::open(&path).unwrap();
        assert_eq!(s.schema_version().unwrap(), 2);
        assert_eq!(
            s.entry(7).unwrap().break_share,
            Some(crate::core::Minutes(20))
        );
    }

    #[test]
    fn entries_with_equal_start_and_end_are_rejected() {
        let s = Store::open_in_memory().unwrap();
        let day = d(2026, 9, 14);
        // A zero-length range reads back as a 24-hour entry, so no write path may store it.
        assert!(matches!(
            s.add_entry(day, t(9, 0), t(9, 0), "Alpha", ""),
            Err(StoreError::Core(CoreError::InvalidRange))
        ));
        assert!(s.entries_on(day).unwrap().is_empty());
        let e = s.add_entry(day, t(9, 0), t(12, 0), "Alpha", "").unwrap();
        assert!(matches!(
            s.update_entry(e.id, t(9, 0), t(9, 0), "Alpha", ""),
            Err(StoreError::Core(CoreError::InvalidRange))
        ));
        assert_eq!(s.entries_on(day).unwrap()[0].end, t(12, 0));
        // Crossing midnight is still allowed.
        s.add_entry(day, t(22, 0), t(2, 0), "Alpha", "").unwrap();
    }

    #[test]
    fn a_corrupt_stored_minute_is_a_query_error_not_a_panic() {
        let s = Store::open_in_memory().unwrap();
        let day = d(2026, 9, 14);
        s.add_entry(day, t(9, 0), t(12, 0), "Alpha", "").unwrap();
        s.conn()
            .execute("UPDATE entries SET end_min = 5000", [])
            .unwrap();
        assert!(matches!(
            s.entries_on(day),
            Err(StoreError::Sqlite(
                rusqlite::Error::FromSqlConversionFailure(..)
            ))
        ));
    }

    #[test]
    fn projects_crud_and_colors() {
        let s = Store::open_in_memory().unwrap();
        let a = s.add_project("Alpha").unwrap();
        let b = s.add_project("Beta").unwrap();
        assert_eq!(a.color_index, 0);
        assert_eq!(b.color_index, 1);
        assert!(matches!(
            s.add_project("Alpha"),
            Err(StoreError::Constraint(_))
        ));
        assert_eq!(s.get_or_create_project("Alpha").unwrap().id, a.id);
        assert_eq!(s.list_projects(false).unwrap().len(), 2);
        s.archive_project("Beta", true).unwrap();
        assert_eq!(s.list_projects(false).unwrap().len(), 1);
        assert_eq!(s.list_projects(true).unwrap().len(), 2);
        s.rename_project("Alpha", "Alpha2").unwrap();
        assert!(s.project_by_name("Alpha2").unwrap().is_some());
        assert!(matches!(
            s.rename_project("Nope", "X"),
            Err(StoreError::NotFound(_))
        ));
    }

    #[test]
    fn entries_crud_overlap_and_ordering() {
        let s = Store::open_in_memory().unwrap();
        let day = d(2026, 9, 14);
        let e2 = s.add_entry(day, t(13, 0), t(17, 0), "Alpha", "pm").unwrap();
        let e1 = s.add_entry(day, t(8, 0), t(12, 0), "Alpha", "am").unwrap();
        let list = s.entries_on(day).unwrap();
        assert_eq!(
            list.iter().map(|e| e.id).collect::<Vec<_>>(),
            vec![e1.id, e2.id]
        );
        assert_eq!(list[0].project, "Alpha");
        assert!(matches!(
            s.add_entry(day, t(11, 0), t(14, 0), "Beta", ""),
            Err(StoreError::Core(CoreError::Overlap))
        ));
        // update can move within own slot
        let e1b = s
            .update_entry(e1.id, t(8, 30), t(12, 0), "Beta", "am2")
            .unwrap();
        assert_eq!(e1b.project, "Beta");
        assert!(matches!(
            s.update_entry(e1.id, t(8, 0), t(14, 0), "Beta", ""),
            Err(StoreError::Core(CoreError::Overlap))
        ));
        s.delete_entry(e2.id).unwrap();
        assert_eq!(s.entries_on(day).unwrap().len(), 1);
        assert!(matches!(s.delete_entry(999), Err(StoreError::NotFound(_))));
        assert_eq!(s.last_used_project().unwrap().as_deref(), Some("Beta"));
        assert_eq!(
            s.entries_in(d(2026, 9, 1), d(2026, 9, 30)).unwrap().len(),
            1
        );
    }

    #[test]
    fn day_kinds_and_entry_constraint() {
        let s = Store::open_in_memory().unwrap();
        let day = d(2026, 9, 14);
        s.set_day_kind(day, &DayKind::Vacation).unwrap();
        assert_eq!(s.stored_kind(day).unwrap(), Some(DayKind::Vacation));
        assert!(matches!(
            s.add_entry(day, t(8, 0), t(9, 0), "Alpha", ""),
            Err(StoreError::Constraint(_))
        ));
        s.set_day_kind(day, &DayKind::Work).unwrap();
        assert_eq!(s.stored_kind(day).unwrap(), None);
        s.add_entry(day, t(8, 0), t(9, 0), "Alpha", "").unwrap();
        assert!(matches!(
            s.set_day_kind(day, &DayKind::Sick),
            Err(StoreError::Constraint(_))
        ));
        let lbl = DayKind::Absence {
            label: "training".into(),
        };
        s.set_day_kind(d(2026, 9, 15), &lbl).unwrap();
        assert_eq!(s.stored_kind(d(2026, 9, 15)).unwrap(), Some(lbl));
    }

    #[test]
    fn days_in_assembles_effective_kinds() {
        let s = Store::open_in_memory().unwrap();
        let cal = HolidayCalendar::new(vec![]);
        s.add_entry(d(2026, 12, 24), t(8, 0), t(12, 0), "Alpha", "")
            .unwrap();
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
        s.clock_in(d(2026, 9, 15), t(8, 12), "Alpha").unwrap();
        let sess = s.session().unwrap().unwrap();
        assert_eq!(sess.start, t(8, 12));
        // The clock-in records what is being worked on, so clocking out needs no guess.
        assert_eq!(sess.project.as_deref(), Some("Alpha"));
        assert!(matches!(
            s.clock_in(d(2026, 9, 15), t(9, 0), "Beta"),
            Err(StoreError::Constraint(_))
        ));
        s.clear_session().unwrap();
        assert!(s.session().unwrap().is_none());
    }

    #[test]
    fn clock_out_books_the_session_project_and_clears_the_session() {
        let s = Store::open_in_memory().unwrap();
        let day = d(2026, 9, 15);
        // Nothing open: there is nothing to book.
        assert!(matches!(
            s.clock_out(at(day, 9, 0), ""),
            Err(StoreError::Constraint(_))
        ));
        s.clock_in(day, t(8, 12), "Alpha").unwrap();
        let e = s
            .clock_out(at(day, 10, 30), "review")
            .unwrap()
            .expect("booked");
        assert_eq!((e.date, e.start, e.end), (day, t(8, 12), t(10, 30)));
        assert_eq!(e.project, "Alpha");
        assert_eq!(e.comment, "review");
        assert!(s.session().unwrap().is_none());
        assert_eq!(s.entries_on(day).unwrap().len(), 1);
    }

    #[test]
    fn clock_out_without_a_session_project_falls_back_to_the_last_used_one() {
        let s = Store::open_in_memory().unwrap();
        let day = d(2026, 9, 15);
        s.add_entry(d(2026, 9, 14), t(9, 0), t(10, 0), "Beta", "")
            .unwrap();
        s.clock_in(day, t(8, 0), "Alpha").unwrap();
        // A session left behind by an older `tk`, which did not record a project.
        s.conn()
            .execute("UPDATE session SET project_id = NULL", [])
            .unwrap();
        assert_eq!(
            s.clock_out(at(day, 9, 0), "")
                .unwrap()
                .expect("booked")
                .project,
            "Beta"
        );

        // With no entry to learn from either, the clock-out says so and keeps the session.
        let s = Store::open_in_memory().unwrap();
        s.clock_in(day, t(8, 0), "Alpha").unwrap();
        s.conn()
            .execute("UPDATE session SET project_id = NULL", [])
            .unwrap();
        let err = s.clock_out(at(day, 9, 0), "").unwrap_err();
        assert!(
            matches!(&err, StoreError::Constraint(m) if m.contains("no project")),
            "{err}"
        );
        assert!(s.session().unwrap().is_some(), "the session is untouched");
        assert!(s.entries_on(day).unwrap().is_empty());
    }

    #[test]
    fn switch_project_books_the_old_one_and_opens_the_new_session() {
        let s = Store::open_in_memory().unwrap();
        let day = d(2026, 9, 15);
        s.clock_in(day, t(8, 12), "Alpha").unwrap();
        let (e, sess) = s
            .switch_project(at(day, 10, 30), "Beta", "morning")
            .unwrap();
        assert_eq!(
            (e.project.as_str(), e.start, e.end),
            ("Alpha", t(8, 12), t(10, 30))
        );
        assert_eq!(e.comment, "morning");
        // The new session starts exactly where the booked entry ends.
        assert_eq!((sess.date, sess.start), (day, t(10, 30)));
        assert_eq!(sess.project.as_deref(), Some("Beta"));

        // Switching to what is already running is refused, and changes nothing.
        let err = s.switch_project(at(day, 11, 0), "Beta", "").unwrap_err();
        assert!(
            matches!(&err, StoreError::Constraint(m) if m.contains("already on")),
            "{err}"
        );
        assert_eq!(s.entries_on(day).unwrap().len(), 1);
        assert_eq!(s.session().unwrap().unwrap().start, t(10, 30));

        // Switching in the same minute as the clock-in: the entry is one minute long
        // and the next session starts after it, so the two can never overlap.
        let (e2, sess2) = s.switch_project(at(day, 10, 30), "Gamma", "").unwrap();
        assert_eq!((e2.start, e2.end), (t(10, 30), t(10, 31)));
        assert_eq!(sess2.start, t(10, 31));
        assert_eq!(sess2.project.as_deref(), Some("Gamma"));
        let list = s.entries_on(day).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list[0].end <= list[1].start, "{list:?}");

        // Clocking out inside the minute the switch skipped forward to books that one
        // minute, not a shift running all the way back round the clock.
        let e3 = s.clock_out(at(day, 10, 30), "").unwrap().expect("booked");
        assert_eq!((e3.start, e3.end), (t(10, 31), t(10, 32)));
        assert_eq!(e3.duration(), crate::core::Minutes(1));

        // Nothing open at all is a plain "not clocked in".
        s.clear_session().unwrap();
        assert!(matches!(
            s.switch_project(at(day, 12, 0), "Beta", ""),
            Err(StoreError::Constraint(_))
        ));
    }

    #[test]
    fn a_run_of_same_minute_switches_books_one_minute_each() {
        // Every same-minute switch puts the next session one minute ahead of the wall
        // clock. Each of them must still book its own minute, in order and without
        // overlapping — never a shift running back round the clock.
        let s = Store::open_in_memory().unwrap();
        let day = d(2026, 9, 15);
        let now = at(day, 9, 35);
        s.clock_in(day, t(9, 35), "A").unwrap();
        s.switch_project(now, "B", "").unwrap();
        s.switch_project(now, "C", "").unwrap();
        let last = s.clock_out(now, "").unwrap().expect("booked");
        assert_eq!(
            (last.project.as_str(), last.start, last.end),
            ("C", t(9, 37), t(9, 38))
        );
        let list = s.entries_on(day).unwrap();
        assert_eq!(
            list.iter()
                .map(|e| (e.project.as_str(), e.start, e.end))
                .collect::<Vec<_>>(),
            vec![
                ("A", t(9, 35), t(9, 36)),
                ("B", t(9, 36), t(9, 37)),
                ("C", t(9, 37), t(9, 38)),
            ]
        );
        assert!(s.session().unwrap().is_none());
    }

    #[test]
    fn an_overnight_session_books_the_whole_night() {
        // The wall clock is on the next day, so an end before the start is a genuine
        // crossing: the store must not mistake it for the same-minute case.
        let s = Store::open_in_memory().unwrap();
        let day = d(2026, 9, 15);
        s.clock_in(day, t(0, 10), "Alpha").unwrap();
        let e = s
            .clock_out(at(d(2026, 9, 16), 0, 9), "")
            .unwrap()
            .expect("booked");
        assert_eq!((e.date, e.start, e.end), (day, t(0, 10), t(0, 9)));
        assert_eq!(e.duration(), crate::core::Minutes(1439));
        // A clock-out dated before the session is a broken clock, not an entry.
        s.clock_in(day, t(8, 0), "Alpha").unwrap();
        let err = s.clock_out(at(d(2026, 9, 14), 8, 30), "").unwrap_err();
        assert!(
            matches!(&err, StoreError::Constraint(m) if m.contains("before the session date")),
            "{err}"
        );
        assert!(s.session().unwrap().is_some());
    }

    #[test]
    fn a_switch_across_midnight_opens_the_session_on_the_next_day() {
        let s = Store::open_in_memory().unwrap();
        let day = d(2026, 9, 15);
        s.clock_in(day, t(23, 59), "Alpha").unwrap();
        let (e, sess) = s
            .switch_project(at(d(2026, 9, 16), 0, 0), "Beta", "")
            .unwrap();
        assert_eq!((e.date, e.start, e.end), (day, t(23, 59), t(0, 0)));
        assert!(e.crosses_midnight());
        // The new session belongs to the day the entry ended on, not the one it began on.
        assert_eq!((sess.date, sess.start), (d(2026, 9, 16), t(0, 0)));
        assert_eq!(sess.project.as_deref(), Some("Beta"));
    }

    #[test]
    fn clocking_out_on_another_project_books_the_override() {
        let s = Store::open_in_memory().unwrap();
        let day = d(2026, 9, 15);
        s.clock_in(day, t(8, 0), "Alpha").unwrap();
        let e = s
            .clock_out_with(at(day, 9, 0), Some("Other"), "wrong project")
            .unwrap()
            .expect("booked");
        assert_eq!(e.project, "Other");
        assert_eq!(e.comment, "wrong project");
        assert!(s.session().unwrap().is_none());
        assert_eq!(s.entries_on(day).unwrap()[0].project, "Other");
    }

    #[test]
    fn break_shares_are_stored_per_entry_and_survive_an_edit() {
        let s = Store::open_in_memory().unwrap();
        let day = d(2026, 9, 15);
        let a = s.add_entry(day, t(8, 0), t(12, 0), "Alpha", "am").unwrap();
        let b = s.add_entry(day, t(12, 0), t(17, 0), "Beta", "pm").unwrap();
        // A new entry is unassigned: the default rule decides what it pays.
        assert_eq!(a.break_share, None);
        assert_eq!(b.break_share, None);
        s.set_break_shares(&[(a.id, Some(Minutes(30))), (b.id, None)])
            .unwrap();
        let list = s.entries_on(day).unwrap();
        assert_eq!(list[0].break_share, Some(Minutes(30)));
        assert_eq!(list[1].break_share, None);
        // Editing the times of an entry is not a change of its share.
        let a2 = s
            .update_entry(a.id, t(8, 30), t(12, 0), "Alpha", "am")
            .unwrap();
        assert_eq!(a2.break_share, Some(Minutes(30)));
        // Clearing is a share of `None`, not a zero.
        s.set_break_shares(&[(a.id, None)]).unwrap();
        assert_eq!(s.entry(a.id).unwrap().break_share, None);
        s.set_break_shares(&[(a.id, Some(Minutes::ZERO))]).unwrap();
        assert_eq!(s.entry(a.id).unwrap().break_share, Some(Minutes::ZERO));
        // An unknown id is refused, and the whole batch is rolled back with it:
        // the shares of a session are only meaningful together.
        let err = s
            .set_break_shares(&[(b.id, Some(Minutes(9))), (999, None)])
            .unwrap_err();
        assert!(matches!(err, StoreError::NotFound(_)), "{err}");
        assert_eq!(s.entry(b.id).unwrap().break_share, None);
    }

    #[test]
    fn a_break_books_the_work_so_far_and_keeps_the_project() {
        let s = Store::open_in_memory().unwrap();
        let day = d(2026, 9, 15);
        // Nothing running: there is no break to take.
        assert!(matches!(
            s.take_break(at(day, 9, 0), ""),
            Err(StoreError::Constraint(_))
        ));
        s.clock_in(day, t(8, 12), "Alpha").unwrap();
        let e = s.take_break(at(day, 12, 3), "morning").unwrap();
        assert_eq!((e.date, e.start, e.end), (day, t(8, 12), t(12, 3)));
        assert_eq!(
            (e.project.as_str(), e.comment.as_str()),
            ("Alpha", "morning")
        );
        // The session stays, paused on the project to come back to, from the
        // minute the work stopped.
        let sess = s.session().unwrap().expect("the session is kept");
        assert_eq!(sess.state, SessionState::Break);
        assert_eq!((sess.date, sess.start), (day, t(12, 3)));
        assert_eq!(sess.project.as_deref(), Some("Alpha"));
        // A second break changes nothing.
        let err = s.take_break(at(day, 12, 30), "").unwrap_err();
        assert!(
            matches!(&err, StoreError::Constraint(m) if m.contains("already on break")),
            "{err}"
        );
        assert_eq!(s.entries_on(day).unwrap().len(), 1);
        assert_eq!(s.session().unwrap().unwrap().start, t(12, 3));
    }

    #[test]
    fn resuming_goes_back_to_the_remembered_project_or_another_one() {
        let s = Store::open_in_memory().unwrap();
        let day = d(2026, 9, 15);
        s.clock_in(day, t(8, 12), "Alpha").unwrap();
        s.take_break(at(day, 12, 3), "").unwrap();
        let sess = s.resume(at(day, 12, 45), None).unwrap();
        assert_eq!(sess.state, SessionState::Working);
        assert_eq!((sess.date, sess.start), (day, t(12, 45)));
        assert_eq!(sess.project.as_deref(), Some("Alpha"));
        // Not on a break any more, so there is nothing to resume.
        let err = s.resume(at(day, 13, 0), None).unwrap_err();
        assert!(
            matches!(&err, StoreError::Constraint(m) if m.contains("not on break")),
            "{err}"
        );
        // Coming back on another project is a resume too.
        s.take_break(at(day, 14, 0), "").unwrap();
        let sess = s.resume(at(day, 14, 20), Some("Beta")).unwrap();
        assert_eq!(sess.project.as_deref(), Some("Beta"));
        assert_eq!(sess.start, t(14, 20));
        // Clocking in on a break resumes it rather than complaining that a
        // session is already open.
        s.take_break(at(day, 15, 0), "").unwrap();
        s.clock_in(day, t(15, 30), "Gamma").unwrap();
        let sess = s.session().unwrap().unwrap();
        assert_eq!(sess.state, SessionState::Working);
        assert_eq!(
            (sess.project.as_deref(), sess.start),
            (Some("Gamma"), t(15, 30))
        );
        // A switch is not the way back from a break: it says which key is.
        s.take_break(at(day, 16, 0), "").unwrap();
        let err = s.switch_project(at(day, 16, 5), "Delta", "").unwrap_err();
        assert!(
            matches!(&err, StoreError::Constraint(m) if m.contains("on break")),
            "{err}"
        );
    }

    #[test]
    fn a_break_taken_after_midnight_books_the_night_and_pauses_on_the_next_day() {
        let s = Store::open_in_memory().unwrap();
        let day = d(2026, 9, 15);
        s.clock_in(day, t(23, 0), "Alpha").unwrap();
        // Half past midnight: a genuine crossing, not the same-minute case.
        let e = s.take_break(at(d(2026, 9, 16), 0, 30), "night").unwrap();
        assert_eq!((e.date, e.start, e.end), (day, t(23, 0), t(0, 30)));
        assert!(e.crosses_midnight());
        assert_eq!(e.duration(), Minutes(90));
        // The break belongs to the day the entry ended on, from the minute the
        // work stopped, still on the project to come back to.
        let sess = s.session().unwrap().expect("the session is kept");
        assert_eq!(sess.state, SessionState::Break);
        assert_eq!((sess.date, sess.start), (d(2026, 9, 16), t(0, 30)));
        assert_eq!(sess.project.as_deref(), Some("Alpha"));
        // Coming back opens the next session after the break, on the new day.
        let back = s.resume(at(d(2026, 9, 16), 1, 0), None).unwrap();
        assert_eq!((back.date, back.start), (d(2026, 9, 16), t(1, 0)));
        let e2 = s
            .clock_out(at(d(2026, 9, 16), 2, 0), "")
            .unwrap()
            .expect("booked");
        assert_eq!(
            (e2.date, e2.start, e2.end),
            (d(2026, 9, 16), t(1, 0), t(2, 0))
        );
    }

    #[test]
    fn clocking_out_on_a_break_clears_it_without_booking() {
        let s = Store::open_in_memory().unwrap();
        let day = d(2026, 9, 15);
        s.clock_in(day, t(8, 12), "Alpha").unwrap();
        s.take_break(at(day, 12, 3), "").unwrap();
        assert!(s.clock_out(at(day, 12, 45), "").unwrap().is_none());
        assert!(s.session().unwrap().is_none());
        // Only the work before the break is on the books.
        let list = s.entries_on(day).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].start, list[0].end), (t(8, 12), t(12, 3)));
    }

    #[test]
    fn the_work_around_a_break_is_two_sessions() {
        use crate::core::{HolidayCalendar, Rules, TodayCtx, day_stats, default_tiers};
        let s = Store::open_in_memory().unwrap();
        let day = d(2026, 9, 15);
        s.clock_in(day, t(8, 0), "Alpha").unwrap();
        s.take_break(at(day, 12, 0), "").unwrap();
        s.resume(at(day, 12, 45), None).unwrap();
        let e = s.clock_out(at(day, 17, 0), "").unwrap().expect("booked");
        assert_eq!((e.start, e.end), (t(12, 45), t(17, 0)));
        let rules = Rules {
            daily_target: Minutes(468),
            tiers: default_tiers(),
            start_date: day,
            initial_balance: Minutes::ZERO,
        };
        let cal = HolidayCalendar::new(vec![]);
        let dd = s.days_in(day, day, &cal).unwrap().remove(0);
        let st = day_stats(
            &dd,
            &rules,
            &cal,
            &TodayCtx {
                today: d(2026, 9, 16),
                clocked_in: false,
            },
        );
        // The 45 minutes off split the day: 4:00 and 4:15, each over three
        // hours, so each loses 18 rather than the 48 of one long session.
        assert_eq!(st.gross, Minutes(495));
        assert_eq!(st.gaps, Minutes(45));
        assert_eq!(st.deduction, Minutes(36));
        assert_eq!(st.net, Minutes(459));
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

    #[test]
    fn write_operations_are_transactional_against_concurrent_writers() {
        use std::time::Duration;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tk.db");
        let a = Store::open(&path).unwrap();
        let b = Store::open(&path).unwrap();
        // b must fail fast instead of waiting out the default 5s busy timeout.
        b.conn().busy_timeout(Duration::ZERO).unwrap();

        let day = d(2026, 9, 14);

        // Connection A holds the write lock via a raw immediate transaction,
        // simulating another `tk` process mid check-then-act.
        a.conn().execute_batch("BEGIN IMMEDIATE").unwrap();
        let err = b.add_entry(day, t(8, 0), t(9, 0), "Alpha", "").unwrap_err();
        assert!(matches!(err, StoreError::Sqlite(_)));
        // A's transaction must not have been left half-open by B's failed attempt.
        a.conn().execute_batch("COMMIT").unwrap();

        // Once A releases the lock, B's write path succeeds normally.
        b.add_entry(day, t(8, 0), t(9, 0), "Alpha", "").unwrap();
        assert_eq!(b.entries_on(day).unwrap().len(), 1);
    }
}
