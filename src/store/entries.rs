use chrono::{NaiveDate, NaiveTime};
use rusqlite::params;

use super::{Store, StoreError, StoreResult, date_str, parse_date, time_from_min};
use crate::core::{DayKind, Entry, check_overlap, check_range, minutes_of};

const SELECT: &str = "SELECT e.id, e.date, e.start_min, e.end_min, p.name, e.comment
                      FROM entries e JOIN projects p ON p.id = e.project_id";

fn row_to_entry(r: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    Ok(Entry {
        id: r.get(0)?,
        date: parse_date(&r.get::<_, String>(1)?)?,
        start: time_from_min(r.get(2)?)?,
        end: time_from_min(r.get(3)?)?,
        project: r.get(4)?,
        comment: r.get(5)?,
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
        self.in_write_tx(|| {
            self.ensure_work_day(date)?;
            // No write path may store a zero-length entry: it would read back as 24 hours.
            check_range(start, end)?;
            check_overlap(&self.entries_on(date)?, start, end, None)?;
            let p = self.get_or_create_project(project)?;
            self.conn().execute(
                "INSERT INTO entries (date, start_min, end_min, project_id, comment) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![date_str(date), minutes_of(start), minutes_of(end), p.id, comment.trim()],
            )?;
            self.entry(self.conn().last_insert_rowid())
        })
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
            self.conn().execute(
                "UPDATE entries SET start_min = ?2, end_min = ?3, project_id = ?4, comment = ?5 WHERE id = ?1",
                params![id, minutes_of(start), minutes_of(end), p.id, comment.trim()],
            )?;
            self.entry(id)
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
