//! The session protocol — the only vocabulary spoken between the turn
//! runner / input surfaces and the session service.
//!
//! Corresponds to pi's `packages/protocol` (much smaller: our events are
//! in-process, no CBOR framing yet). Two halves:
//!
//! - [`SessionEvent`] — *inputs* to the session service. The turn
//!   runner emits streaming/commit events; the UI emits user text.
//! - [`Change`] — *outputs*. What a consumed event changed, so a
//!   renderer reacts (redraw, scroll pin) without learning *how* the
//!   state mutated internally.

use crate::ai::types::{StopReason, Usage};
use crate::entry::Entry;

/// Events the session service consumes, in order.
///
/// A headless frontend feeds `Submit` + drains the resulting `Change`s;
/// the TUI feeds the same events from keystrokes. No surface special
/// cases.
#[derive(Debug, Clone)]
pub enum SessionEvent {
    // ---- turn streaming (from the turn runner thread) ----
    /// Streaming text delta.
    Delta(String),
    /// Streaming reasoning delta.
    ReasoningDelta(String),
    /// A tool started executing. `args` is the raw JSON argument string;
    /// `intent` is the model's one-line statement of purpose.
    ToolStart {
        call_id: String,
        name: String,
        args: String,
        intent: String,
    },
    /// A tool finished executing.
    ToolFinish {
        call_id: String,
        name: String,
        ok: bool,
        result: String,
    },
    /// An error occurred (not persisted; memory stream only).
    Error(String),
    /// A turn finished: usage for billing, stop_reason to distinguish
    /// interrupts. Finalizes the streaming slots into an Assistant entry.
    TurnDone(Usage, StopReason),
    /// Turn finalized: the whole round's entries ship for one-shot
    /// persistence (the session verifies them against its pending list).
    Commit(Vec<Entry>),
    /// The turn runner thread is exiting.
    Done,

    // ---- user input (from any surface) ----
    /// The user submitted a message; starts a turn (or is rejected while
    /// one streams — the emitter checks `busy()` first, not the session).
    Submit(String),
    /// /name: rename the session. Persists a Name marker under the
    /// current leaf (branches inherit names, siblings never see them).
    NameMarker(String),
    /// /cd landed: persist the migrated working directory under `seq`.
    /// Pure bookkeeping — no transcript change.
    SetCwd { seq: i64, path: String },
    /// Compaction finished on the background thread: apply the fork.
    /// `entries` is the marker to persist; `ctx` replaces the live chat
    /// replica; the token stats are display-only.
    Compaction {
        entries: Vec<Entry>,
        ctx: crate::ai::types::Context,
        tokens_before: usize,
        tokens_after: usize,
    },
}

/// What happened after a mutation — enough for a renderer to react
/// (redraw / scroll pin / cost tick) without exposing internals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Transcript contents changed (echo, commit, navigation...).
    Transcript,
    /// Streaming buffers advanced (Delta/ReasoningDelta/Done). Renderers
    /// redraw the live slots but never touch the transcript.
    Stream,
    /// A turn completed (TurnDone): usage landed on the tracker; the
    /// live streaming slot folded into a final entry.
    TurnDone,
    /// A tool finished locally (ToolFinish): the result exists in the
    /// transcript but has NOT been sent back to the model yet. Surfaces
    /// use this as the git-refresh checkpoint — tools are the only
    /// things that can move the working tree, and this is the earliest
    /// moment new state is observable.
    ToolActivity,
    /// The current session changed (new session created, resumed...).
    Session,
    /// Nothing observable happened (e.g. store unavailable, no-op).
    None,
}

/// What the user is waiting on right now — the one live row at the bottom
/// of the transcript.
///
/// A dedicated type (rather than a bare string) so the renderer can style
/// each state differently — and so future ones can be added without the
/// transcript learning a new special case.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub enum LiveActivity {
    /// Nothing in flight (or the reply's text has already started, which
    /// needs no placeholder — the text itself occupies the row).
    #[default]
    Idle,
    /// The server has begun emitting reasoning: it is thinking.
    Thinking,
    /// A tool is executing. Carries the model's own one-line explanation of
    /// what the call is for; empty when the model offered none.
    Tool { intent: String },
}

/// The in-flight streaming slots, read-only. The renderer snapshots
/// these each frame; the session owns the buffers.
#[derive(Debug, Default, Clone)]
pub struct StreamView {
    /// A turn is currently streaming.
    pub active: bool,
    /// Reply currently streaming (the in-progress slot). Swapped for an
    /// Assistant entry once final; never touches the DB meanwhile.
    pub text: String,
    /// Reasoning buffer currently streaming (in-progress slot; enters an
    /// entry when final).
    pub reasoning: String,
    /// Whether content has started (reasoning slot stops updating after).
    pub reasoning_done: bool,
    /// What the user is waiting on (drives the live row).
    pub live: LiveActivity,
}

impl StreamView {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty() && self.reasoning.is_empty() && !self.active
    }
}
