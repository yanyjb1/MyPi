//! TUI layer — ratatui + crossterm.
//!
//! Division of labor (aligned with pi's packages/tui + modes/interactive):
//! - Pure logic (no terminal dependency, unit-testable):
//!   `text` / `editor` / `keys` / `theme` / `layout` /
//!   `undo` / `history` / `path` (including the completion popup state machine)
//! - Components (render state into Lines): `components/*`
//! - Orchestration (event loop + rendering): `app` / `view` / `events`
pub mod app;
mod transcript;
pub mod completion;
pub mod editor;
pub mod components;
pub mod highlight;
pub mod keys;
pub mod leaf;
pub mod layout;
pub mod text;
pub mod signal;
pub mod theme;
pub mod view;
pub mod zones;
pub mod zones_impl;
