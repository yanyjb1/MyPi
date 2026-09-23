//! Browser providers: the per-command implementations behind `engine.rs`.
//! Page-state commands (open/read/screenshot) stay in the engine — they
//! are one-liners; interaction and capture grow here.

pub(crate) mod act;
pub(crate) mod net;

pub(crate) use act::cmd_act;
