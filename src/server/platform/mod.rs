//! platform — the layer that asks the operating system: time, stamps, directories, environment, log,
//! the shell and the process tree.

pub mod env;
pub mod log;
pub mod os;
pub mod proc;
pub mod shell;
pub mod stamp;
pub mod time;
