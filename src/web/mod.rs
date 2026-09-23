//! Web domain — search / fetch / browser tools and the CDP plumbing they
//! share. Directory layout mirrors pi's harness/tools pattern: one file per
//! tool or engine, `mod.rs` as the only import surface for the outside.
//!
//! Dependency direction (enforced by visibility, `pub(super)` below):
//!   tools.rs → web::{search, fetch, browser} → web::session → web::cdp
//! `bing.rs` is pure HTTP (no browser); `session.rs` is the single owner of
//! attach-or-launch, socket reconnection, and readiness polling.

pub mod browser;
pub mod cdp;
pub mod ddg;
pub mod fetch;
mod html;
pub mod bing;
mod search;
mod session;
pub mod url;

// Tool-layer facade: everything `crate::agent::tools` may touch.
pub use browser::{browser, parse_browser_args};
pub use fetch::{fetch, parse_fetch_args};
pub use search::{parse_search_args, render, search, SearchArgs, SearchHit};
