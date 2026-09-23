//! Fetch tool — URL to model-facing markdown, tiered by transport.

mod engine;
pub mod providers;

pub use engine::{fetch, html_to_markdown, parse_fetch_args, FetchArgs};
pub(crate) use engine::is_js_shell;
