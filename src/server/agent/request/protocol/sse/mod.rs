//! Server-sent events: one frame at a time. Pure mechanics: no protocol knowledge.

mod event;
mod frame;
mod read;

pub use event::{SseEvent, read_event};
pub use frame::{Frame, frame};
pub use read::read;
