//! The server side — everything that is *not* the terminal: api, storage, agent, config.
//! Directions are one-way; the guard table in `src/lib.rs` enforces them.

pub mod agent;
pub mod api;
pub mod config;
pub mod storage;
