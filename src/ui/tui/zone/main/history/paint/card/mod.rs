//! Tool cards: one file per tool family, and the shared rendering.

pub(crate) mod alias;
pub(crate) mod bash;
pub(crate) mod context;
pub(crate) mod draft;
pub(crate) mod edit;
pub(crate) mod fallback;
pub(crate) mod file;
pub(crate) mod kairos;
pub(crate) mod mcp;
pub(crate) mod memo;
pub(crate) mod task;
pub(crate) mod todo;
pub(crate) mod tree;
pub(crate) mod web;

mod frame;

pub(crate) use frame::*;
