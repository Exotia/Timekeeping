//! The store worker: a plain thread owning `Ctx` (Store + Config), plus the async port
//! that carries its replies back into the tui-realm event listener.

use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::{self, JoinHandle};

use chrono::{Datelike, Local, NaiveDate};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tuirealm::event::Event;
use tuirealm::listener::{PollAsync, PortResult};

use super::msg::{DayData, MonthData, StatsData, StoreCmd, StoreReply, UserEvent};
use crate::cli::Ctx;
use crate::core::{
    DayKind, Minutes, TodayCtx, clock_out_end, day_stats, is_working_day, running_balance,
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

fn handle(ctx: &Ctx, cmd: StoreCmd) -> anyhow::Result<StoreReply> {
    let now = Local::now().naive_local();
    let today = now.date();
    Ok(match cmd {
        StoreCmd::LoadMonth { year, month } => {
            let (from, to) = month_range(year, month);
            let cal = ctx.config.calendar();
            let session = ctx.store.session()?;
            let days = ctx.store.days_in(from, to, &cal)?;
            let (y0, y1) = (
                NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(today.year(), 12, 31).unwrap(),
            );
            let vacation_used_this_year = ctx
                .store
                .stored_kinds_in(y0, y1)?
                .iter()
                .filter(|(d, k)| **k == DayKind::Vacation && is_working_day(**d))
                .count() as u32;
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
        StoreCmd::LoadStats { from, to } => StoreReply::Stats(StatsData {
            from,
            to,
            days: ctx.store.days_in(from, to, &ctx.config.calendar())?,
            projects: ctx.store.list_projects(true)?,
        }),
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
        StoreCmd::ClockIn(date, t) => {
            ctx.store.clock_in(date, t)?;
            StoreReply::Changed(format!("Clocked in at {}", t.format("%H:%M")))
        }
        StoreCmd::ClockOut { project, comment } => {
            let s = ctx
                .store
                .session()?
                .ok_or_else(|| anyhow::anyhow!("not clocked in"))?;
            let project = match project.or(ctx.store.last_used_project()?) {
                Some(p) => p,
                None => anyhow::bail!("no project yet; add the entry from the day editor"),
            };
            let end = clock_out_end(s.start, now.time());
            let e = ctx
                .store
                .add_entry(s.date, s.start, end, &project, &comment)?;
            ctx.store.clear_session()?;
            StoreReply::Changed(format!(
                "Clocked out: {}–{} {} ({})",
                e.start.format("%H:%M"),
                e.end.format("%H:%M"),
                e.project,
                e.duration()
            ))
        }
        StoreCmd::ClearSession => {
            ctx.store.clear_session()?;
            StoreReply::Changed(String::new())
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
        Ok(self
            .rx
            .recv()
            .await
            .map(|r| Event::User(UserEvent::Store(r))))
    }
}
