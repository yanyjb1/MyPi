//! ipc — the wire between the two mypi processes: one byte stream, one JSON frame per line.
//! The contract is shared; the client and server halves meet only here.

pub mod client;
pub mod contract;
pub mod endpoint;
pub mod line;
pub mod server;
pub(crate) mod stream;
pub mod transport;
