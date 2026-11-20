// The wire contract: frames and the text shape only. No payload types, no names.

pub mod error;
pub mod message;
pub mod text;

pub use error::ErrorPayload;
pub use message::Frame;
pub use text::Text;
