//! The terminal front end: ratatui + crossterm on top of `ui::session`.
//!
//! `action` captures user input, `app` holds the ledger, `zone` draws, `run` is the frame loop,
//! `lang` holds display text.

pub mod action;
pub mod app;
mod clipboard;
mod config;
pub mod lang;
mod run;
mod signal;
pub mod term;
pub mod text;
pub mod theme;
pub mod zone;

pub use config::TodoFold;
pub use run::run;
