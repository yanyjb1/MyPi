//! Profile discovery and resolution. A profile is a directory under `profiles/`.

mod discover;
mod resolve;
mod types;

pub use discover::{list, locate};
pub use resolve::{compaction, resolve, system};
pub use types::{DEFAULT_PROFILE, ResolvedProfile};