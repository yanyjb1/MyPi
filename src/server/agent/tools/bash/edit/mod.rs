//! edit: replace one unique run of text, or one range of lines.

mod tool;

pub(crate) use tool::{TOOL, exact_text, mass_text, parse_range};
