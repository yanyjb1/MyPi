//! UI components — each renders one piece of state into ratatui
//! `Line` lists.
//!
//! Convention: components are **stateless** functional rendering (data
//! in, Lines out); state lives in `app.rs`. That keeps components
//! testable without spawning a terminal.

pub mod markdown;
pub mod input;
pub mod popup;
pub mod reserved;
pub mod statusline;
pub mod tree_picker;
