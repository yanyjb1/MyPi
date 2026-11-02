//! Terminal text primitives: cell width, truncation, wrapping, and the shared card frame.

pub(crate) mod card;
pub(crate) mod glyphs;

mod truncate;
mod width;
mod wrap;

pub use truncate::take_width;
pub use width::{char_width, display_width};
pub use wrap::{Wrapped, wrap};
