//! The writer: the one thing that writes. `Writer` is a handle; one thread owns the connection,
//! so exactly one writer ever touches the database.

pub(crate) mod handle;
pub(crate) mod job;
mod thread;

pub(crate) use handle::Store;
pub use handle::Writer;
