//! Search tool — routing over the keyless engines.
//!
//! | file | role |
//! |---|---|
//! | `engine.rs` | arguments, engine chain / aggregate, notes, rendering |
//! | `providers/` | one module per engine + the shared two-tier fetch contract |
//! | `query.rs` | structured query: parse, reformat per engine, lenient filter |
//! | `hit.rs` | the result model, dedup key, date/age math |
//! | `http.rs` | plain-HTTP transport (browser-shaped, byte-capped) |
//!
//! Everything below `engine.rs` is engine-local: routing, merging and output
//! shaping never leak into a provider, and a provider never sees another one.

mod engine;
mod hit;
mod http;
mod providers;
mod query;

pub use engine::{Engine, SearchArgs, SearchOutcome, parse_search_args, render, search};
pub use hit::SearchHit;
