//! TUI runtime: tokio runtime, store worker thread, tui-realm application and main loop.

pub mod components;
pub mod ids;
pub mod model;
pub mod msg;
pub mod theme;
pub mod view;
pub mod worker;

use std::time::Duration;

use chrono::Local;
use tuirealm::application::{Application, PollStrategy};
use tuirealm::listener::EventListenerCfg;
use tuirealm::subscription::{EventClause, Sub, SubClause};
use tuirealm::terminal::{CrosstermTerminalAdapter, TerminalAdapter};

use crate::cli::Ctx;
use ids::Id;
use model::{Model, Screen};
use msg::{Msg, StoreReply, UserEvent};

pub fn run(ctx: Ctx) -> anyhow::Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()?;
    let _guard = rt.enter();

    let theme = theme::Theme::from_config(&ctx.config);
    // Kept before the worker takes ownership of `ctx`: the settings overlay writes
    // `config.toml` back into this directory.
    let home = ctx.home.clone();
    let rules = ctx.config.rules();
    let cal = ctx.config.calendar();
    let vacation_allowance = ctx.config.vacation_days_per_year;
    let hours = ctx.config.hours_format();

    let (reply_tx, reply_rx) = tokio::sync::mpsc::unbounded_channel::<StoreReply>();
    let (worker, worker_handle) = worker::spawn_worker(ctx, reply_tx);

    let cfg = EventListenerCfg::default()
        .with_handle(rt.handle().clone())
        .async_crossterm_input_listener(Duration::ZERO, 3)
        .add_async_port(
            Box::new(worker::StorePort::new(reply_rx)),
            Duration::ZERO,
            1,
        )
        .tick_interval(Duration::from_secs(1))
        .async_tick(true);
    let mut app: Application<Id, Msg, UserEvent> = Application::init(cfg);
    app.mount(
        Id::Bridge,
        Box::new(components::bridge::Bridge::default()),
        vec![
            Sub::new(EventClause::Tick, SubClause::Always),
            Sub::new(
                EventClause::User(UserEvent::Store(StoreReply::Changed(String::new()))),
                SubClause::Always,
            ),
        ],
    )?;
    app.mount(
        Id::Month,
        Box::new(components::month::MonthScreen::default()),
        vec![],
    )?;
    app.mount(
        Id::Confirm,
        Box::new(components::confirm::ConfirmDialog::default()),
        vec![],
    )?;
    app.mount(
        Id::Help,
        Box::new(components::help::HelpOverlay::default()),
        vec![],
    )?;
    app.mount(
        Id::Stats,
        Box::new(components::stats::StatsScreen::default()),
        vec![],
    )?;
    app.mount(
        Id::Day,
        Box::new(components::day::DayScreen::default()),
        vec![],
    )?;
    app.mount(
        Id::Projects,
        Box::new(components::projects::ProjectsScreen::default()),
        vec![],
    )?;
    app.active(&Id::Month)?;

    // Restore the terminal on panic so the shell is usable.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm_restore();
        default_hook(info);
    }));

    let mut terminal = CrosstermTerminalAdapter::new()?;
    terminal.enable_raw_mode()?;
    terminal.enter_alternate_screen()?;

    let now = Local::now().naive_local();
    let mut model = Model {
        app,
        home,
        terminal: Some(terminal),
        theme,
        rules,
        cal,
        vacation_allowance,
        screen: Screen::Month,
        selected: now.date(),
        today: now.date(),
        now: now.time(),
        month: None,
        day: None,
        stats: None,
        day_cursor: 0,
        confirm: None,
        help: false,
        status: None,
        quit: false,
        redraw: true,
        worker,
        stats_range: msg::RangeKind::Month,
        form: None,
        settings: None,
        clock_picker: None,
        hours,
        stats_anchor: now.date(),
        // The chart opens on the per-period bars; `r` is what asks for the
        // running balance, and it then holds for the session.
        chart_mode: msg::ChartMode::default(),
        split_followup: None,
        break_split: None,
        projects: None,
        projects_cursor: 0,
        prompt: None,
    };
    model.load_month();

    while !model.quit {
        match model
            .app
            .tick(PollStrategy::Once(Duration::from_millis(10)))
        {
            Err(e) => model.set_status(format!("event error: {e}"), true),
            Ok(msgs) => {
                for m in msgs {
                    model.update(m);
                }
            }
        }
        if model.redraw {
            model.view();
            model.redraw = false;
        }
    }
    // Let the store thread drain everything queued before `Shutdown` (a mutating key and `q`
    // can arrive in the same tick batch). The terminal goes back to normal first, so that a
    // slow or wedged drain is not spent staring at the alternate screen — and so that the
    // join error below is printed into a usable shell.
    model.worker.send(msg::StoreCmd::Shutdown);
    if let Some(mut t) = model.terminal.take() {
        t.restore()?;
    }
    if worker_handle.join().is_err() {
        anyhow::bail!("the store thread died; recent changes may not have been saved");
    }
    Ok(())
}

fn crossterm_restore() -> std::io::Result<()> {
    use tuirealm::ratatui::crossterm::{execute, terminal};
    terminal::disable_raw_mode()?;
    execute!(std::io::stdout(), terminal::LeaveAlternateScreen)
}
