//! The prefetch layer: the laid segments around the screen, whole segments in and out.

pub(crate) mod color;
mod range;
mod run;
mod segment;
mod trim;
pub(crate) mod walk;

pub(crate) use range::{Aim, Range, Start};
pub(crate) use run::{Head, Placed, Run};
