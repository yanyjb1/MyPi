//! TUI layer — ratatui + crossterm.
//!
//! Division of labor (aligned with pi's packages/tui + modes/interactive):
//! - Pure logic (no terminal dependency, unit-testable):
//!   `text` / `editor` / `keys` / `theme` / `layout` /
//!   `undo` / `history` / `path` (including the completion popup state machine)
//! - Components (render state into Lines): `components/*`
//! - Orchestration (event loop + rendering): `app` / `view` / `events`
pub mod app;
pub mod components;
pub mod editor;
pub mod events;
pub mod history;
pub mod keys;
pub mod leaf;
pub mod layout;
pub mod paste;
pub mod path;
pub mod text;
pub mod theme;
pub mod undo;
pub mod view;
pub mod zones;
pub mod zones_impl;
