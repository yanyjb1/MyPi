//! bash: the command tool, the gate every command passes, and the runner.

pub(crate) mod admit;
pub(crate) mod guard;
pub(crate) mod segment;
pub(crate) mod shell;
mod tool;

pub(crate) use tool::TOOL;
