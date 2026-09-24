//! The server side — everything that is *not* the terminal.
//!
//! Layout (mirrors pi's `packages/protocol` + `packages/server` split,
//! flattened into one module since our scope is smaller):
//!
//! - [`events`]  — the **protocol**: every observable thing the session
//!   service can announce, plus the changes it reports back.
//! - [`session`] — the session service state machine. It consumes
//!   `SessionEvent`s in order and emits `Change`s; it knows nothing
//!   about terminals. Run it headless and it is already a working
//!   agent backend.
//! - [`turn`]    — the turn runner: one background thread per turn,
//!   translating the agent loop's raw callbacks into `SessionEvent`s.
//!
//! The TUI (`crate::tui`) is a subscriber: it feeds input events in,
//! drains the change stream, and draws. It holds no conversation state
//! of its own.

pub mod artifacts;
pub mod compaction;
pub mod events;
pub mod profile;
pub mod session;
pub mod turn;

pub use events::{Change, SessionEvent, StreamView};
pub use session::{Session, SessionState};
