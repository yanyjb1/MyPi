//! The chat zone: the host of history, input and reserved.

mod arbitrate;
mod desk;
mod geometry;
mod history;
mod input;
mod keys;
mod num;
mod pane;
mod reserved;
mod zone;

pub use input::Draft;
pub use zone::MainZone;
