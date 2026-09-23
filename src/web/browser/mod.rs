//! Browser tool — drive and inspect a page in the session's work tab.

mod engine;
pub mod providers;

pub use engine::{browser, parse_browser_args, BrowserArgs};
