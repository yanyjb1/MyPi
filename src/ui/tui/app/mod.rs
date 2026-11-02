//! The app: the registry, the ledger, the only exit to the session.

mod app;
mod lifecycle;
mod startup;

pub use app::{App, Input};
pub use startup::Startup;
