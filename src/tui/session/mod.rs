//! Terminal-session lifecycle — owning the store, the event loop, and
//! the frame pump. app.rs answers "what does this keypress mean"; this
//! domain owns everything from process start to cursor placement.

mod r#loop;
pub mod signal;
pub mod view;

pub use r#loop::run_tui;
