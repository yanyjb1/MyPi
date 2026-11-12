//! Tools. The root declares modules; everything is reached by its own path.

mod bash;
pub mod contract;
pub(crate) mod engine;
mod kairos;
pub(crate) mod mcp;
pub(crate) mod memo;
pub(crate) mod registry;
pub(crate) mod spec;
pub(crate) mod subagent;
mod summary;
pub(crate) mod workspace;

pub(crate) mod web;
