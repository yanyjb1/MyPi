//! session — the front end's half of the wire: one connection, two windows, one outbox.

mod actions;
mod client;
mod content;
mod packet;
mod session;
mod structure;
mod vocabulary;

pub use packet::Packet;
pub use session::Session;
pub use structure::{Structure, Turn};
pub use vocabulary::{ModelId, Retention};
