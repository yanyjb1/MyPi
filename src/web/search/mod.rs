//! Search tool — engine routing over site providers.

mod engine;
pub mod providers;

pub use engine::{parse_search_args, render, search, SearchArgs, SearchHit};
