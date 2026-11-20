// The compiled-in display text: one JSON file per language, looked up by a caller-chosen key.

mod render;
mod table;

pub use render::render;
pub use table::{languages, resolve, text};
