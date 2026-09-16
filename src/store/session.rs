use std::cmp::Ordering;

use chrono::{Days, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use rusqlite::{OptionalExtension, params};

use super::{Store, StoreError, StoreResult, date_str, parse_date, time_from_min};
use crate::core::{Entry, clock_out_end, minutes_of};

/// Entries are stored at minute granularity, so the seconds `Local::now()` carries
/// are dropped before anything is compared or written.
fn to_minute(t: NaiveTime) -> NaiveTime {
    NaiveTime::from_hms_opt(t.hour(), t.minute(), 0).unwrap_or(t)
}

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

    /// Open a session on `project`, creating that project if it is new.
    pub fn clock_in(&self, date: NaiveDate, start: NaiveTime, project: &str) -> StoreResult<()> {
        self.in_write_tx(|| self.clock_in_locked(date, start, project))
    }

    /// Close the open session at the wall clock `now`, booking the time to the
    /// session's project.
    ///
    /// `now` is a whole date-time because only the date tells an overnight session
    /// apart from a clock-out that lands before the session start.
    pub fn clock_out(&self, now: NaiveDateTime, comment: &str) -> StoreResult<Entry> {
        self.clock_out_with(now, None, comment)
    }

    /// [`Store::clock_out`], with `project` booking the entry to something other than
    /// the session's project — what `tk out -p NAME` does to correct a clock-in.
    pub fn clock_out_with(
        &self,
        now: NaiveDateTime,
        project: Option<&str>,
        comment: &str,
    ) -> StoreResult<Entry> {
        self.in_write_tx(|| self.clock_out_locked(now, project, comment))
    }

    /// Book the running session and open a new one on `project`, in one transaction.
    ///
    /// The new session starts exactly where the booked entry ends, so the zero-length
    /// rule's extra minute can never make the two overlap.
    pub fn switch_project(
        &self,
        now: NaiveDateTime,
        project: &str,
        comment: &str,
    ) -> StoreResult<(Entry, Session)> {
        self.in_write_tx(|| {
            let s = self.require_session()?;
            let target = project.trim();
            if s.project.as_deref() == Some(target) {
                return Err(StoreError::Constraint(format!("already on {target}")));
            }
            let entry = self.clock_out_locked(now, None, comment)?;
            // An entry that ran past midnight ends on the following day, and that is
            // the day the next session belongs to.
            let date = if entry.crosses_midnight() {
                entry
                    .date
                    .checked_add_days(Days::new(1))
                    .ok_or_else(|| StoreError::Constraint("date out of range".into()))?
            } else {
                entry.date
            };
            self.clock_in_locked(date, entry.end, target)?;
            let session = self.session()?.expect("just clocked in");
            Ok((entry, session))
        })
    }

    pub fn clear_session(&self) -> StoreResult<()> {
        self.conn()
            .execute("DELETE FROM session WHERE id = 1", [])?;
        Ok(())
    }

    /// The open session, or the error every clock-out path reports for "nothing running".
    fn require_session(&self) -> StoreResult<Session> {
        self.session()?
            .ok_or_else(|| StoreError::Constraint("not clocked in".into()))
    }

    /// [`Store::clock_in`] without the transaction around it.
    fn clock_in_locked(&self, date: NaiveDate, start: NaiveTime, project: &str) -> StoreResult<()> {
        if let Some(s) = self.session()? {
            return Err(StoreError::Constraint(format!(
                "already clocked in since {} {}",
                date_str(s.date),
                s.start.format("%H:%M")
            )));
        }
        let p = self.get_or_create_project(project)?;
        self.conn().execute(
            "INSERT INTO session (id, date, start_min, project_id) VALUES (1, ?1, ?2, ?3)",
            params![date_str(date), minutes_of(start), p.id],
        )?;
        Ok(())
    }

    /// [`Store::clock_out_with`] without the transaction around it.
    fn clock_out_locked(
        &self,
        now: NaiveDateTime,
        project: Option<&str>,
        comment: &str,
    ) -> StoreResult<Entry> {
        let s = self.require_session()?;
        // A session from before clock-in recorded a project still has to be bookable.
        let project = match project
            .map(str::to_string)
            .or(s.project)
            .or(self.last_used_project()?)
        {
            Some(p) => p,
            None => {
                return Err(StoreError::Constraint(
                    "no project; use --project NAME".into(),
                ));
            }
        };
        let end = to_minute(now.time());
        let end = match now.date().cmp(&s.date) {
            // Still the day the session began on. The wall clock can sit at — or even
            // before — the session start, because every same-minute switch opens the
            // next session one minute further ahead of it; clamping to the start makes
            // such a clock-out the shortest possible entry rather than a shift that
            // runs all the way back round the clock.
            Ordering::Equal => clock_out_end(
                s.start,
                if minutes_of(end) < minutes_of(s.start) {
                    s.start
                } else {
                    end
                },
            ),
            // A later day is a genuine overnight session: core reads an end at or
            // before the start as the midnight wrap.
            Ordering::Greater => end,
            Ordering::Less => {
                return Err(StoreError::Constraint(
                    "clock-out time is before the session date".into(),
                ));
            }
        };
        let entry = self.add_entry_locked(s.date, s.start, end, &project, comment)?;
        self.clear_session()?;
        Ok(entry)
    }
}
