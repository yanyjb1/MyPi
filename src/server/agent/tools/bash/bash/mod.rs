//! bash: the one command tool, its guard and its runner.

pub(crate) mod guard;
pub(crate) mod shell;
mod tool;

pub(crate) use tool::{TOOL, expand_alias, rewrite_refs};
