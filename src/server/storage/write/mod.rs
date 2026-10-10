//! The write half: everything that lands goes through here — `batches`, `compaction`, `sessions`,
//! `artifacts`, `shared`. The **when** belongs to the writer's inlet.

pub(crate) mod artifacts;
pub(crate) mod batches;
pub(crate) mod compaction;
pub(crate) mod sessions;
pub(crate) mod shared;
