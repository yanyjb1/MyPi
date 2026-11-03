//! Colors: one live accent, one fixed palette.

mod accent;
mod contrast;

pub mod fixed;

pub use accent::Accent;
pub use contrast::contrast_text_on;
