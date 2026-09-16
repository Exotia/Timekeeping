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

/// What the open session is doing: running the clock on its project, or paused
/// on it with the work so far already booked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionState {
    Working,
    Break,
}

impl SessionState {
    /// The spelling the `session.state` column uses.
    pub fn as_str(self) -> &'static str {
        match self {
            SessionState::Working => "working",
            SessionState::Break => "break",
        }
    }

    /// A stored state; anything unknown reads as `Working`, which is what every
    /// session was before the state existed.
    pub fn parse(s: &str) -> SessionState {
        match s {
            "break" => SessionState::Break,
            _ => SessionState::Working,
        }
    }

    pub fn is_break(self) -> bool {
        matches!(self, SessionState::Break)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    pub date: NaiveDate,
    pub start: NaiveTime,
    pub project: Option<String>,
    /// While `Break`, `date`/`start` are when the break began and `project` is
    /// the project to come back to.
    pub state: SessionState,
}

impl Store {
    pub fn session(&self) -> StoreResult<Option<Session>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT s.date, s.start_min, p.name, s.state FROM session s
                 LEFT JOIN projects p ON p.id = s.project_id WHERE s.id = 1",
                [],
                |r| {
                    Ok(Session {
                        date: parse_date(&r.get::<_, String>(0)?)?,
                        start: time_from_min(r.get(1)?)?,
                        project: r.get(2)?,
                        state: SessionState::parse(&r.get::<_, String>(3)?),
                    })
                },
            )
            .optional()?)
    }

    /// Open a session on `project`, creating that project if it is new.
    ///
    /// On a break this is a [`Store::resume`] instead: clocking in is what the
    /// user does to go back to work, whether or not they think of it as ending
    /// a break, and `project` is then what they come back on.
    pub fn clock_in(&self, date: NaiveDate, start: NaiveTime, project: &str) -> StoreResult<()> {
        self.in_write_tx(|| {
            if self
                .session()?
                .is_some_and(|s| s.state == SessionState::Break)
            {
                self.resume_locked(NaiveDateTime::new(date, start), Some(project))?;
                return Ok(());
            }
            self.clock_in_locked(date, start, project)
        })
    }

    /// Book the work done so far and pause on the same project.
    ///
    /// The entry is booked exactly as a clock-out would book it, so the session
    /// date rules are the same; instead of clearing the session the row is kept,
    /// on the project to come back to, from the minute the work stopped.
    pub fn take_break(&self, now: NaiveDateTime, comment: &str) -> StoreResult<Entry> {
        self.in_write_tx(|| {
            let s = self.require_session()?;
            if s.state == SessionState::Break {
                return Err(StoreError::Constraint("already on break".into()));
            }
            let entry = self.book_running(now, None, comment)?;
            // The break begins where the booked work ends, not at the bare wall
            // clock: a break taken in the same minute as the clock-in books one
            // minute of work, and the break cannot start before that minute is
            // over without the next entry overlapping it.
            let date = self.day_after_entry(&entry)?;
            self.conn().execute(
                "UPDATE session SET date = ?1, start_min = ?2, state = 'break' WHERE id = 1",
                params![date_str(date), minutes_of(entry.end)],
            )?;
            Ok(entry)
        })
    }

    /// Go back to work from a break, on `project` or on the remembered one.
    pub fn resume(&self, now: NaiveDateTime, project: Option<&str>) -> StoreResult<Session> {
        self.in_write_tx(|| self.resume_locked(now, project))
    }

    /// Close the open session at the wall clock `now`, booking the time to the
    /// session's project.
    ///
    /// `now` is a whole date-time because only the date tells an overnight session
    /// apart from a clock-out that lands before the session start.
    ///
    /// `None` means the session was on a break: the work had already been booked
    /// when the break began, so ending it clears the session and writes nothing.
    /// One entry point for "stop the clock", whatever state it is in, is why this
    /// is an `Option` rather than a second method the caller has to choose.
    pub fn clock_out(&self, now: NaiveDateTime, comment: &str) -> StoreResult<Option<Entry>> {
        self.clock_out_with(now, None, comment)
    }

    /// [`Store::clock_out`], with `project` booking the entry to something other than
    /// the session's project — what `tk out -p NAME` does to correct a clock-in.
    pub fn clock_out_with(
        &self,
        now: NaiveDateTime,
        project: Option<&str>,
        comment: &str,
    ) -> StoreResult<Option<Entry>> {
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
            if s.state == SessionState::Break {
                return Err(StoreError::Constraint(
                    "on break; use tk in -p PROJECT".into(),
                ));
            }
            if s.project.as_deref() == Some(target) {
                return Err(StoreError::Constraint(format!("already on {target}")));
            }
            let entry = self
                .clock_out_locked(now, None, comment)?
                .expect("a working session books an entry");
            // An entry that ran past midnight ends on the following day, and that is
            // the day the next session belongs to.
            let date = self.day_after_entry(&entry)?;
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

    /// [`Store::resume`] without the transaction around it.
    fn resume_locked(&self, now: NaiveDateTime, project: Option<&str>) -> StoreResult<Session> {
        let s = self.require_session()?;
        if s.state != SessionState::Break {
            return Err(StoreError::Constraint("not on break".into()));
        }
        let project = match project
            .map(str::trim)
            .filter(|p| !p.is_empty())
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
        let p = self.get_or_create_project(&project)?;
        // The work goes on from the wall clock, but never from before the break
        // began: the break starts where the last entry ended, so a resume inside
        // that same minute (or on a clock that has gone backwards) would open a
        // session overlapping the entry already booked.
        let back_at = if NaiveDateTime::new(s.date, s.start) > now {
            NaiveDateTime::new(s.date, s.start)
        } else {
            now
        };
        self.conn().execute(
            "UPDATE session SET date = ?1, start_min = ?2, project_id = ?3, state = 'working'
             WHERE id = 1",
            params![
                date_str(back_at.date()),
                minutes_of(to_minute(back_at.time())),
                p.id
            ],
        )?;
        Ok(self.session()?.expect("the session was just updated"))
    }

    /// The day the session after `entry` belongs to: an entry that ran past
    /// midnight ends on the following day.
    fn day_after_entry(&self, entry: &Entry) -> StoreResult<NaiveDate> {
        if entry.crosses_midnight() {
            entry
                .date
                .checked_add_days(Days::new(1))
                .ok_or_else(|| StoreError::Constraint("date out of range".into()))
        } else {
            Ok(entry.date)
        }
    }

    /// [`Store::clock_out_with`] without the transaction around it.
    fn clock_out_locked(
        &self,
        now: NaiveDateTime,
        project: Option<&str>,
        comment: &str,
    ) -> StoreResult<Option<Entry>> {
        let s = self.require_session()?;
        if s.state == SessionState::Break {
            // The work was booked when the break began; there is nothing left to
            // write, only the paused session to clear.
            self.clear_session()?;
            return Ok(None);
        }
        let entry = self.book_running(now, project, comment)?;
        self.clear_session()?;
        Ok(Some(entry))
    }

    /// Book the running session as an entry, leaving the session row alone.
    ///
    /// The one place a running clock becomes an entry: the clock-out clears the
    /// session afterwards, a switch opens the next one, and a break pauses on the
    /// same project, so all three book the time by exactly the same rules.
    fn book_running(
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
        self.add_entry_locked(s.date, s.start, end, &project, comment)
    }
}
