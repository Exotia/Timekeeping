//! SQLite-backed store.

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
            v => Err(StoreError::Migration(format!(
                "unsupported schema version {v}"
            ))),
        }
    }

    pub fn schema_version(&self) -> StoreResult<i64> {
        let v: String = self.conn.query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |r| r.get(0),
        )?;
        v.parse()
            .map_err(|_| StoreError::Migration(format!("bad schema_version {v}")))
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

pub(crate) fn date_str(d: chrono::NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

pub(crate) fn parse_date(s: &str) -> rusqlite::Result<chrono::NaiveDate> {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
    })
}

pub(crate) fn time_from_min(m: i64) -> chrono::NaiveTime {
    chrono::NaiveTime::from_hms_opt((m / 60) as u32, (m % 60) as u32, 0)
        .expect("stored minutes valid")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{CoreError, DayKind, HolidayCalendar};
    use chrono::{NaiveDate, NaiveTime};

    fn d(y: i32, m: u32, dd: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, dd).unwrap()
    }
    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

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
        s.clock_in(d(2026, 9, 15), t(8, 12)).unwrap();
        let sess = s.session().unwrap().unwrap();
        assert_eq!(sess.start, t(8, 12));
        assert!(matches!(
            s.clock_in(d(2026, 9, 15), t(9, 0)),
            Err(StoreError::Constraint(_))
        ));
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
