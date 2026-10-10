//! The compat switches for OpenAI-compatible providers. The field list follows oh-my-pi's
//! `OpenAICompat`, and behind every switch is one real 400. Layers: `types`, `defaults`, `rules`, `engine`.

pub mod rules;

mod defaults;
mod engine;
mod types;

pub use engine::resolve;
pub use types::*;
