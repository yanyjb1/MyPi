// The wire contract: frames and error shapes only. No payload types, no names.

pub mod error;
pub mod message;

pub use error::{ErrorCode, ErrorPayload};
pub use message::Frame;
