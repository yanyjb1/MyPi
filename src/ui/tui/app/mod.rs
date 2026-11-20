//! The app: the registry, the ledger, the one post box between the zones and the session.

mod app;
mod lifecycle;
mod startup;

pub use app::{App, Input};
pub use startup::Startup;
