//! Per-kind transcript styling. Stateless: data in, Lines out.
//!
//! `chat` is the kind dispatch + the full-transcript convenience render;
//! the card scaffolding and per-kind blocks live in their own files.

pub(crate) mod assistant;
pub(crate) mod cards;
pub(crate) mod chat;
pub(crate) mod system;
