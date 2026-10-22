//! Terminal text primitives: cell width, truncation, wrapping.

mod truncate;
mod width;
mod wrap;

pub use truncate::take_width;
pub use width::{char_width, display_width};
pub use wrap::{Wrapped, wrap};
