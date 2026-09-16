//! The store worker: a plain thread owning `Ctx` (Store + Config), plus the async port
//! that carries its replies back into the tui-realm event listener.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::{self, JoinHandle};

use chrono::{Datelike, Local, NaiveDate};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tuirealm::event::Event;
use tuirealm::listener::{PollAsync, PortError, PortResult};

use super::msg::{DayData, MonthData, StatsData, StoreCmd, StoreReply, UserEvent};
use crate::cli::Ctx;
use crate::core::{
    DayKind, Entry, Minutes, TodayCtx, day_stats, deduction, is_working_day, running_balance,
    session_members,
};

pub struct Worker {
    pub tx: Sender<StoreCmd>,
}

impl Worker {
    /// Queue a command for the store thread. Returns `false` once that thread is gone.
    pub fn send(&self, cmd: StoreCmd) -> bool {
        self.tx.send(cmd).is_ok()
    }
}

/// Spawn the store thread. All database work happens there, never on the UI thread.
///
/// The returned [`JoinHandle`] must be joined after sending [`StoreCmd::Shutdown`], otherwise
/// commands still queued when the user quits are dropped without ever reaching SQLite.
pub fn spawn_worker(ctx: Ctx, reply_tx: UnboundedSender<StoreReply>) -> (Worker, JoinHandle<()>) {
    let (tx, rx) = channel::<StoreCmd>();
    let handle = thread::Builder::new()
        .name("tk-store".into())
        .spawn(move || worker_loop(ctx, rx, reply_tx))
        .expect("spawn worker");
    (Worker { tx }, handle)
}

fn worker_loop(ctx: Ctx, rx: Receiver<StoreCmd>, reply: UnboundedSender<StoreReply>) {
    while let Ok(cmd) = rx.recv() {
        if matches!(cmd, StoreCmd::Shutdown) {
            break;
        }
        let r = handle(&ctx, cmd).unwrap_or_else(|e| StoreReply::Failed(e.to_string()));
        if reply.send(r).is_err() {
            break;
        }
    }
}

/// (first, last) day of the given month.
pub fn month_range(year: i32, month: u32) -> (NaiveDate, NaiveDate) {
    let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    let next = if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
    .unwrap();
    (first, next.pred_opt().unwrap())
}

fn balance_through(
    ctx: &Ctx,
    to: NaiveDate,
    today: NaiveDate,
    clocked_in: bool,
) -> anyhow::Result<Minutes> {
    let rules = ctx.config.rules();
    let cal = ctx.config.calendar();
    if to < rules.start_date {
        return Ok(rules.initial_balance);
    }
    let days = ctx.store.days_in(rules.start_date, to, &cal)?;
    let tc = TodayCtx { today, clocked_in };
    let stats: Vec<_> = days
        .iter()
        .map(|d| day_stats(d, &rules, &cal, &tc))
        .collect();
    Ok(running_balance(&stats, &rules))
}

/// (1 January, 31 December) of `year`.
fn year_range(year: i32) -> (NaiveDate, NaiveDate) {
    (
        NaiveDate::from_ymd_opt(year, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(year, 12, 31).unwrap(),
    )
}

/// Vacation days that consume the allowance: stored `Vacation` kinds on working days.
fn count_vacation(ctx: &Ctx, from: NaiveDate, to: NaiveDate) -> anyhow::Result<u32> {
    Ok(ctx
        .store
        .stored_kinds_in(from, to)?
        .iter()
        .filter(|(d, k)| **k == DayKind::Vacation && is_working_day(**d))
        .count() as u32)
}

fn vacation_working_days_in_year(ctx: &Ctx, year: i32) -> anyhow::Result<u32> {
    let (y0, y1) = year_range(year);
    count_vacation(ctx, y0, y1)
}

/// The reply a booked entry deserves: the message, plus how many projects its
/// session spans and what that session loses to the break.
///
/// Both figures decide whether it is worth asking the user how to split that
/// deduction, and both are read off the day as it now stands — the entry is
/// already on the books when this runs.
fn booked(ctx: &Ctx, e: &Entry, message: String) -> anyhow::Result<StoreReply> {
    let day = ctx.store.entries_on(e.date)?;
    let intervals: Vec<(i32, i32)> = day.iter().map(Entry::interval).collect();
    let group = session_members(&intervals)
        .into_iter()
        .find(|g| g.iter().any(|&i| day[i].id == e.id))
        .unwrap_or_default();
    let span = {
        let s = group.iter().map(|&i| intervals[i].0).min().unwrap_or(0);
        let end = group.iter().map(|&i| intervals[i].1).max().unwrap_or(0);
        end - s
    };
    let mut projects: Vec<&str> = group.iter().map(|&i| day[i].project.as_str()).collect();
    projects.sort_unstable();
    projects.dedup();
    Ok(StoreReply::Booked {
        entry_id: e.id,
        date: e.date,
        session_projects: projects.len(),
        deduction: deduction(Minutes(span), &ctx.config.rules().tiers),
        message,
    })
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
            let (y0, y1) = year_range(today.year());
            let vacation_used_this_year = count_vacation(ctx, y0, y1)?;
            StoreReply::Month(MonthData {
                year,
                month,
                days,
                balance_before: balance_through(
                    ctx,
                    from.pred_opt().unwrap(),
                    today,
                    session.is_some(),
                )?,
                balance_total: balance_through(ctx, today, today, session.is_some())?,
                session,
                projects: ctx.store.list_projects(false)?,
                last_used_project: ctx.store.last_used_project()?,
                vacation_used_this_year,
            })
        }
        StoreCmd::LoadDay(date) => StoreReply::Day(DayData {
            day: ctx
                .store
                .days_in(date, date, &ctx.config.calendar())?
                .remove(0),
            projects: ctx.store.list_projects(false)?,
        }),
        StoreCmd::LoadStats { from, to, year } => {
            let session = ctx.store.session()?.is_some();
            StoreReply::Stats(StatsData {
                from,
                to,
                days: ctx.store.days_in(from, to, &ctx.config.calendar())?,
                projects: ctx.store.list_projects(true)?,
                // The allowance is a calendar-year budget, so what is left of it never
                // depends on the range inside the year the user is looking at — but it
                // does follow the year they navigate to.
                vacation_used_year: vacation_working_days_in_year(ctx, year)?,
                session_active: session,
                // What the balance stood at when the range opened — the same walk the
                // month view's `balance_before` makes, from `start_date` to the day
                // before the range.
                carried_in: balance_through(ctx, from.pred_opt().unwrap_or(from), today, session)?,
            })
        }
        StoreCmd::AddEntry {
            date,
            start,
            end,
            project,
            comment,
        } => {
            let e = ctx.store.add_entry(date, start, end, &project, &comment)?;
            StoreReply::Changed(format!(
                "Added {}–{} {}",
                e.start.format("%H:%M"),
                e.end.format("%H:%M"),
                e.project
            ))
        }
        StoreCmd::UpdateEntry {
            id,
            start,
            end,
            project,
            comment,
        } => {
            let e = ctx.store.update_entry(id, start, end, &project, &comment)?;
            StoreReply::Changed(format!(
                "Updated {}–{} {}",
                e.start.format("%H:%M"),
                e.end.format("%H:%M"),
                e.project
            ))
        }
        StoreCmd::DeleteEntry(id) => {
            ctx.store.delete_entry(id)?;
            StoreReply::Changed("Entry deleted".into())
        }
        StoreCmd::SetKind(date, kind) => {
            ctx.store.set_day_kind(date, &kind)?;
            StoreReply::Changed(format!(
                "{date} set to {}",
                kind.display_name().to_lowercase()
            ))
        }
        StoreCmd::ClockIn(date, t, project) => {
            ctx.store.clock_in(date, t, &project)?;
            StoreReply::Changed(format!("Clocked in on {project} at {}", t.format("%H:%M")))
        }
        StoreCmd::Switch { project } => {
            let (e, s) = ctx.store.switch_project(now, &project, "")?;
            StoreReply::Changed(format!(
                "Booked {}–{} {} · now on {}",
                e.start.format("%H:%M"),
                e.end.format("%H:%M"),
                e.project,
                s.project.as_deref().unwrap_or(&project)
            ))
        }
        StoreCmd::ClockOut { project, comment } => {
            match ctx
                .store
                .clock_out_with(now, project.as_deref(), &comment)?
            {
                Some(e) => {
                    let message = format!(
                        "Clocked out: {}–{} {} ({})",
                        e.start.format("%H:%M"),
                        e.end.format("%H:%M"),
                        e.project,
                        e.duration()
                    );
                    booked(ctx, &e, message)?
                }
                // A break was ended: the work before it is already on the books.
                None => StoreReply::Changed("Break ended".into()),
            }
        }
        StoreCmd::SetBreakShares(shares) => {
            ctx.store.set_break_shares(&shares)?;
            StoreReply::Changed("Break split saved".into())
        }
        StoreCmd::Shutdown => StoreReply::Changed(String::new()),
    })
}

/// Async port feeding store replies into the tui-realm event listener.
pub struct StorePort {
    rx: UnboundedReceiver<StoreReply>,
}

impl StorePort {
    pub fn new(rx: UnboundedReceiver<StoreReply>) -> Self {
        Self { rx }
    }
}

#[tuirealm::async_trait]
impl PollAsync<UserEvent> for StorePort {
    async fn poll(&mut self) -> PortResult<Option<Event<UserEvent>>> {
        match self.rx.recv().await {
            Some(r) => Ok(Some(Event::User(UserEvent::Store(r)))),
            // A plain `Ok(None)` retires the port silently; a permanent error retires it
            // too, but surfaces in `Application::tick` so the user is told.
            None => Err(PortError::PermanentError(
                "the store thread stopped sending replies".into(),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::store::Store;
    use chrono::NaiveTime;
    use std::path::Path;

    fn d(y: i32, m: u32, dd: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, dd).unwrap()
    }

    /// A `Ctx` on an in-memory store, tracking from 1 January 2025 with an hour
    /// of overtime carried in from before that.
    fn ctx(home: &Path) -> Ctx {
        let mut config = Config::load_or_create(home).unwrap();
        config.start_date = d(2025, 1, 1);
        config.initial_balance_minutes = 60;
        Ctx {
            home: home.to_path_buf(),
            config,
            store: Store::open_in_memory().unwrap(),
        }
    }

    fn stats(ctx: &Ctx, from: NaiveDate, to: NaiveDate) -> StatsData {
        match handle(
            ctx,
            StoreCmd::LoadStats {
                from,
                to,
                year: from.year(),
            },
        )
        .unwrap()
        {
            StoreReply::Stats(s) => s,
            other => panic!("expected Stats, got {other:?}"),
        }
    }

    /// `LoadStats` carries in the balance the range opens on, so a running chart
    /// goes on from where the ranges before it ended. For a month that is exactly
    /// what the month view knows as `balance_before`.
    #[test]
    fn load_stats_carries_in_the_balance_from_before_the_range() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx(dir.path());
        let t = |h| NaiveTime::from_hms_opt(h, 0, 0).unwrap();
        // Two nine-hour days in June 2025: +24 minutes each on the daily target.
        for day in [d(2025, 6, 2), d(2025, 6, 3)] {
            ctx.store.add_entry(day, t(8), t(17), "Alpha", "").unwrap();
        }
        let july = stats(&ctx, d(2025, 7, 1), d(2025, 7, 31));
        let month = match handle(
            &ctx,
            StoreCmd::LoadMonth {
                year: 2025,
                month: 7,
            },
        )
        .unwrap()
        {
            StoreReply::Month(m) => m,
            other => panic!("expected Month, got {other:?}"),
        };
        assert_eq!(july.carried_in, month.balance_before);
        // Everything before the range is in there: half a year of unbooked work
        // days leaves far less than the hour that was carried into 2025.
        assert!(
            july.carried_in < Minutes(60),
            "{:?} should hold the months before July",
            july.carried_in
        );
        // A range that opens on the start date has nothing before it but the
        // initial balance.
        assert_eq!(
            stats(&ctx, d(2025, 1, 1), d(2025, 1, 31)).carried_in,
            Minutes(60)
        );
    }
}
