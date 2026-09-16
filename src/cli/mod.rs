pub mod commands;

use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

use crate::config::{Config, ConfigError, resolve_home};
use crate::store::Store;

#[derive(Parser, Debug)]
#[command(
    name = "tk",
    version,
    about = "Flexitime tracker. Run without a command to open the TUI."
)]
pub struct Cli {
    /// Data directory (default: $TK_HOME or ~/.local/share/tk)
    #[arg(long, global = true, value_name = "DIR")]
    pub home: Option<PathBuf>,
    #[command(subcommand)]
    pub cmd: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Clock in now
    In {
        /// Replace an existing clock-in
        #[arg(long)]
        force: bool,
        /// Project to work on (default: the last one used)
        #[arg(short, long)]
        project: Option<String>,
    },
    /// Clock out and record the entry
    Out {
        #[arg(short, long)]
        project: Option<String>,
        #[arg(short = 'm', long)]
        comment: Option<String>,
    },
    /// Book the work so far and pause the clock on the same project
    Break {
        /// Comment for the entry being booked
        #[arg(short = 'm', long)]
        comment: Option<String>,
    },
    /// Book the running session and clock in on another project
    Switch {
        /// Project to switch to
        #[arg(short, long)]
        project: String,
        /// Comment for the entry being booked
        #[arg(short = 'm', long)]
        comment: Option<String>,
    },
    /// One-line status for prompts and status bars
    Status,
    /// Add an entry: tk add 2026-09-14 0900-1530 -p Alpha -m "note"
    Add {
        date: String,
        range: String,
        #[arg(short, long)]
        project: String,
        #[arg(short = 'm', long, default_value = "")]
        comment: String,
    },
    /// Set a day kind: work | vacation | flex | holiday | sick | absence
    Day {
        date: String,
        kind: String,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        label: Option<String>,
    },
    /// Manage projects
    Projects {
        #[command(subcommand)]
        action: Option<ProjectAction>,
    },
    /// Show or change core settings (start date, initial balance, daily target,
    /// vacation days, hours format)
    Config {
        /// New start date for the balance (YYYY-MM-DD, today, yesterday, or an offset)
        // A negative day offset (`--start -1`) is a value, not another flag.
        #[arg(long, value_name = "DATE", allow_hyphen_values = true)]
        start: Option<String>,
        /// New carried-over balance at the start date
        // Likewise a negative carried-over balance (`--balance -2:30`).
        #[arg(long, value_name = "±HH:MM", allow_hyphen_values = true)]
        balance: Option<String>,
        /// New daily target
        #[arg(long, value_name = "HH:MM")]
        target: Option<String>,
        /// New yearly vacation allowance in days
        #[arg(long, value_name = "DAYS")]
        vacation: Option<u32>,
        /// How durations are printed: "hm" (07:48) or "decimal" (7.80h)
        #[arg(long, value_name = "hm|decimal")]
        hours: Option<String>,
    },
    /// Copy the database into backups/
    Backup,
    /// Export entries as CSV or JSON
    Export {
        #[arg(long, default_value = "csv")]
        format: String,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ProjectAction {
    List,
    Add { name: String },
    Archive { name: String },
    Unarchive { name: String },
    Rename { old: String, new: String },
}

pub struct Ctx {
    pub home: PathBuf,
    pub config: Config,
    pub store: Store,
}

/// Exit code 2 for config problems, 1 for everything else.
pub struct ExitCode(pub i32);

pub fn open_ctx(home: Option<&Path>) -> Result<Ctx, (ExitCode, anyhow::Error)> {
    let home = resolve_home(home);
    let config = Config::load_or_create(&home).map_err(|e: ConfigError| (ExitCode(2), e.into()))?;
    let store = Store::open(&home.join("tk.db")).map_err(|e| (ExitCode(1), e.into()))?;
    Ok(Ctx {
        home,
        config,
        store,
    })
}
