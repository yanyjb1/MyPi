//! Fetch tool — URL to model-facing markdown, tiered by transport.

mod engine;
pub mod providers;

pub(crate) use engine::is_js_shell;
pub use engine::{FetchArgs, fetch, html_to_markdown, parse_fetch_args};
