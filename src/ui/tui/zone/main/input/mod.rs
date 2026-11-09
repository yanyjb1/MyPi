//! The input zone: editor, history, statusline, submit, command routing.
//! It does not know what sits above it; it only knows when it lent its keys away.

mod boot;
mod claim;
pub mod command;
mod ctx;
mod editor;
mod esc;
mod exit;
mod frame;
mod intent;
mod notice;
mod queue;
mod render;
mod route;
mod semantics;
mod statusline;
mod submit;
mod zone;

pub use ctx::{Ctx, Currency, Data};
pub use exit::ExitRequest;
pub use notice::Notice;
pub use zone::InputZone;
