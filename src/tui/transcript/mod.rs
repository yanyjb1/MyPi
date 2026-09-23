//! Transcript domain — how the conversation becomes paintable blocks.
//!
//! Division:
//! - `blocks`: groups entries into nodes (request+result glue) and renders
//!   each node to width-wrapped rows of exact height;
//! - `cache`: the bounded LRU over rendered blocks + the height roster
//!   that gives O(blocks) scroll math;
//! - `components/` the per-kind look: cards, assistant text, system notices.
//!
//! The layout/orchestration side lives in `super::view`; this module is the
//! "what does one node look like and how tall is it" authority.

pub mod blocks;
pub mod cache;

pub(crate) mod components;
