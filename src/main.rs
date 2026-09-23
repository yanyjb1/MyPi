//! Entry point — the TUI build (the role pi's packages/coding-agent/src/main.ts plays).
//!
//! Configuration lives in ~/.config/mypi/config.yaml (XDG): providers /
//! models / pricing are all external, zero hardcoding in code. The repo
//! ships `models_example.yml` as the template.

use mypi::ai::config::Config;
use mypi::cli::Cli;
use mypi::tui::app;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse(std::env::args().skip(1))?;
    let mut cfg = Config::load()?;
    // --model overrides the default for this session only (never written
    // back; /model is the persistent path).
    let model_override = cli.model.clone();
    if let Some(spec) = &model_override {
        // Validate the id now — fail before the TUI starts, not on the
        // first request.
        cfg.model_by_id(spec)?;
        cfg.default = Some(spec.clone());
    }
    app::run_tui(cfg, cli)
}
