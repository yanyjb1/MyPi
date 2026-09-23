//! Entry point — the TUI build (the role pi's packages/coding-agent/src/main.ts plays).
//!
//! Configuration lives in models.yml: providers / models / pricing are all external, zero hardcoding in code.
//! To switch models or change prices: edit models.yml, no recompile needed.

use mypi::ai::config::Config;
use mypi::tui::app;

fn main() -> anyhow::Result<()> {
    let cfg = Config::load()?;
    app::run_tui(cfg)
}
