//! The display layer: the sight line, its budget, and the rows it takes.

pub(crate) mod budget;
pub(crate) mod focus;
pub(crate) mod scroll;
pub(crate) mod state;
pub(crate) mod window;

pub(crate) use state::{Dir, Scene, View};
