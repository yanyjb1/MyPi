// Tools. The root declares modules; everything is reached by its own path.

mod cd;
pub mod contract;
pub(crate) mod engine;
mod fs;
mod kairos;
pub(crate) mod mcp;
pub(crate) mod memo;
pub(crate) mod registry;
#[cfg(feature = "browser")]
pub(crate) mod search;
mod shell;
pub(crate) mod spec;
pub(crate) mod subagent;
mod summary;
pub(crate) mod workspace;

#[cfg(feature = "browser")]
pub(crate) mod web;
