//! Server-sent events: one frame at a time. Pure mechanics: no protocol knowledge.

mod frame;
mod read;

pub use frame::{Frame, frame, worker_error};
pub use read::read;
