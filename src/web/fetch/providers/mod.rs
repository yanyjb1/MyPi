//! Fetch providers: one file per transport. A provider turns a URL into
//! raw HTML (or an error) and knows nothing about tiers or output
//! shaping — that is `engine.rs`'s job.

pub(crate) mod browser;
pub(crate) mod direct;
pub(crate) mod site;

pub(crate) use browser::fetch_via_browser;
pub(crate) use direct::looks_bot_blocked;
pub(crate) use direct::fetch_direct;
