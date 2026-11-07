//! The prefetch layer: the laid segments around the sight line.

pub(crate) mod color;
mod run;
mod segment;
mod trim;
pub(crate) mod walk;

pub(crate) use run::Run;
pub(crate) use segment::At;
