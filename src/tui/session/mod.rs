//! Terminal-session lifecycle — owning the store, the event loop, and
//! the frame pump. app.rs answers "what does this keypress mean"; this
//! domain owns everything from process start to cursor placement.

mod db;
pub mod signal;
mod r#loop;

pub(crate) use db::{db_path, migrate_legacy_db};
pub use r#loop::run_tui;
