// Git, read two ways: what the statusline draws of a working tree, and what the file tools hand over when a session commits.

mod repo;
mod run;
mod stage;
mod status;

pub use status::{GitStatus, count_porcelain, snapshot};

pub(crate) use repo::{facts, ignored, relative, root, tracked};
pub(crate) use stage::{add, record};
pub(crate) use status::changed;
