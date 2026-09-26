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
//! - [`hub`]     — the multi-session host: one process, N conversations,
//!   one SQLite connection each, no shared state.
//! - [`log`]     — in-memory narration (retries, timeouts). About **us**, not
//!   about the conversation, so it is never persisted.
//!
//! Below them, the server's domains: [`entry`] (protocol data model),
//! [`store`] (SQLite), [`ai`] (the model gateway), [`agent`] (the execution
//! engine). They used to sit at the crate root; a surface that wants to talk
//! to this server imports `mypi::server`, and nothing else moves.
//!
//! The TUI (`crate::tui`) is a subscriber: it feeds input events in,
//! drains the change stream, and draws. It holds no conversation state
//! of its own.
//!
//! # What a front end gets (the outward surface)
//!
//! Nothing here is TUI-shaped: a web UI, a chat bridge or a test harness uses
//! the same calls.
//!
//! **Send in** — `Session::submit(text)` starts a round (false when one is
//! already streaming); `submit` is the only way user text enters. Everything
//! else is protocol: `Session::ingest(SessionEvent)` / `drain(rx)`.
//!
//! **Take out** (deltas are already decoded: a subscriber never parses JSON):
//! - `Delta` / `ReasoningDelta` — reply text and thinking, as content;
//! - `ToolStart` / `ToolFinish` — the call the model asked for and its result;
//! - `TurnDone(usage, stop_reason)` / `Error` — how the round ended;
//! - `Session::status() -> RunState` — one coarse value (`Thinking` /
//!   `Replying` / `Tool{intent}` / `Idle`) for clients that render no
//!   transcript, so they never decode tool payloads to say "正在调用工具";
//! - `Session::transcript()` / `stream_view()` / `spend()` — render state;
//! - `Session::replay_round(session, round) -> Replay` — a stored round's
//!   request, rebuilt from the database alone.
//!
//! Delivery cadence is a server-side switch (`config.yaml → app.streaming`,
//! or `MYPI_STREAM_MODE`): `immediate` forwards each chunk, `buffered` hands
//! the turn over in one piece. Same final state either way.

// 服务端自己的领域：协议数据模型、持久化、模型网关、执行引擎。
// 以前散在 crate 根目录（`crate::entry` / `crate::store` / `crate::ai` /
// `crate::agent`），现在全部收在 `server` 下：服务端换了目录，前端只认协议。
pub mod agent;
pub mod ai;
pub mod entry;
pub mod log;
pub mod store;

#[cfg(test)]
pub(crate) mod test_gateway;

pub mod hub;

pub mod commands;
pub mod compaction;
pub mod daemon;
pub mod events;
pub mod profile;
pub mod prompts;
pub mod session;
pub mod turn;
pub mod wire;

/// The daemon's unix socket: `$XDG_RUNTIME_DIR/mypi.sock` (falls back to
/// /tmp when XDG_RUNTIME_DIR is unset, e.g. some CI boxes).
pub fn socket_path() -> std::path::PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("mypi.sock")
}

pub use events::{Change, RunState, SessionEvent, StreamView};
pub use wire::ClientConn;
pub use hub::{SessionHub, SessionSpec};
pub use session::{Replay, Session, SessionState};
