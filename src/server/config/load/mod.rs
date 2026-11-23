//! Loading: layers, per-key merge, warnings, bootstrap, and the `Config` handle.

mod aliases;
mod api;
mod check;
mod effective;
mod entry;
mod source;
mod types;

#[cfg(feature = "web")]
pub use types::BrowserOptions;
pub use types::{Config, Effective};
