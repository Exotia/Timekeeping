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
                        start: time_from_min(r.get(1)?)?,
                        project: r.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn clock_in(&self, date: NaiveDate, start: NaiveTime) -> StoreResult<()> {
        self.in_write_tx(|| {
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
        })
    }

    pub fn clear_session(&self) -> StoreResult<()> {
        self.conn()
            .execute("DELETE FROM session WHERE id = 1", [])?;
        Ok(())
    }
}
