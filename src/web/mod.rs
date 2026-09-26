//! Web domain — search / fetch / browser tools over shared browser
//! infrastructure. Layout: one directory per tool, `engine.rs` routes and
//! falls back between `providers/` (one file per site/transport), shared
//! state and helpers live in `utils/`.
//!
//! Dependency direction (one way, enforced by visibility):
//!   tools.rs → web::{search, fetch, browser} (this facade)
//!     → */engine.rs (routing, fallback, output shaping)
//!       → */providers/* (per-site / per-transport implementations)
//!         → utils::{session, cdp, html, url}

pub mod browser;
pub mod fetch;
pub mod search;
mod utils;

// Tool-layer facade: everything `crate::server::agent::tools` may touch.
pub use browser::{browser, parse_browser_args};
pub use fetch::{fetch, parse_fetch_args};
pub use search::{SearchArgs, SearchHit, parse_search_args, render, search};
