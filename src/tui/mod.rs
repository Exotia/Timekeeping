//! TUI (implemented from Task 12 on)
pub mod theme;
pub mod view;

use crate::cli::Ctx;

pub fn run(_ctx: Ctx) -> anyhow::Result<()> {
    anyhow::bail!("TUI not implemented yet; use `tk --help`")
}
