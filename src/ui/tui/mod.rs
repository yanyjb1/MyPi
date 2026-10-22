//! The terminal front end: ratatui + crossterm on top of `ui::session`.
//!
//! `action` captures user input, `event` names everything else, `app` holds the ledger,
//! `zone` draws. The frame loop is the next wave.

pub mod action;
pub mod app;
pub mod event;
pub mod term;
pub mod text;
pub mod theme;
pub mod zone;
