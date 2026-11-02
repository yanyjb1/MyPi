//! The terminal front end: ratatui + crossterm on top of `ui::session`.
//!
//! `action` captures user input, `app` holds the ledger, `zone` draws, `run` is the frame loop.

pub mod action;
pub mod app;
mod config;
mod run;
mod signal;
pub mod term;
pub mod text;
pub mod theme;
pub mod zone;

pub use config::{ThemeConfig, TodoFold, UsageAlign};
pub use run::run;
