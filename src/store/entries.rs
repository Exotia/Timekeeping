use chrono::{NaiveDate, NaiveTime};
use rusqlite::params;

use super::{Store, StoreError, StoreResult, date_str, parse_date, time_from_min};
use crate::core::import::ImportRow;
use crate::core::{CoreError, DayKind, Entry, Minutes, check_overlap, check_range, minutes_of};

const SELECT: &str = "SELECT e.id, e.date, e.start_min, e.end_min, p.name, e.comment, e.break_share
                      FROM entries e JOIN projects p ON p.id = e.project_id";

fn row_to_entry(r: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: r.get(0)?,
        date: parse_date(&r.get::<_, String>(1)?)?,
        start: time_from_min(r.get(2)?)?,
        end: time_from_min(r.get(3)?)?,
        project: r.get(4)?,
        comment: r.get(5)?,
        break_share: r.get::<_, Option<i64>>(6)?.map(|m| Minutes(m as i32)),
    })
}

impl Store {
    pub fn entries_on(&self, date: NaiveDate) -> StoreResult<Vec<Entry>> {
        let mut st = self.conn().prepare(&format!(
            "{SELECT} WHERE e.date = ?1 ORDER BY e.start_min, e.id"
        ))?;
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
        self.in_write_tx(|| self.add_entry_locked(date, start, end, project, comment))
    }

    /// The body of [`Store::add_entry`], without the transaction around it, so that a
    /// caller already inside one (the clock-out path) can book an entry as part of it.
    pub(super) fn add_entry_locked(
        &self,
        date: NaiveDate,
        start: NaiveTime,
        end: NaiveTime,
        project: &str,
        comment: &str,
    ) -> StoreResult<Entry> {
        self.ensure_work_day(date)?;
        // No write path may store a zero-length entry: it would read back as 24 hours.
        check_range(start, end)?;
        check_overlap(&self.entries_on(date)?, start, end, None)?;
        let p = self.get_or_create_project(project)?;
        // `break_share` is left to its column default (NULL): a new entry is
        // unassigned, and the default rule decides what it pays.
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
        self.in_write_tx(|| {
            let existing = self.entry(id)?;
            check_range(start, end)?;
            check_overlap(&self.entries_on(existing.date)?, start, end, Some(id))?;
            let p = self.get_or_create_project(project)?;
            // The share is not the form's to change: editing the times of an
            // entry keeps whatever share it was given (see `set_break_shares`).
            self.conn().execute(
                "UPDATE entries SET start_min = ?2, end_min = ?3, project_id = ?4, comment = ?5 WHERE id = ?1",
                params![id, minutes_of(start), minutes_of(end), p.id, comment.trim()],
            )?;
            self.entry(id)
        })
    }

    /// Set (or clear, with `None`) the explicit break share of several entries
    /// at once.
    ///
    /// One transaction over the lot: the box that edits the shares of a whole
    /// session either saves all of them or none, so no session is ever left
    /// half-assigned. An id that does not exist is reported and nothing is
    /// written — the shares of a session are only meaningful together.
    pub fn set_break_shares(&self, shares: &[(i64, Option<Minutes>)]) -> StoreResult<()> {
        self.in_write_tx(|| {
            for (id, share) in shares {
                let n = self.conn().execute(
                    "UPDATE entries SET break_share = ?2 WHERE id = ?1",
                    params![id, share.map(|m| m.0)],
                )?;
                if n == 0 {
                    return Err(StoreError::NotFound(format!("entry {id}")));
                }
            }
            Ok(())
        })
    }

    pub fn delete_entry(&self, id: i64) -> StoreResult<()> {
        let n = self
            .conn()
            .execute("DELETE FROM entries WHERE id = ?1", [id])?;
        if n == 0 {
            return Err(StoreError::NotFound(format!("entry {id}")));
        }
        Ok(())
    }
}

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
