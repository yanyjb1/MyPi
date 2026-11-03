//! Tool cards: one file per tool family, and the shared rendering.

pub(crate) mod bash;
pub(crate) mod edit;
pub(crate) mod fallback;
pub(crate) mod file;
pub(crate) mod todo;
pub(crate) mod web;

mod frame;

pub(crate) use frame::*;
