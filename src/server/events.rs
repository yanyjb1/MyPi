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

use crate::server::ai::types::{StopReason, Usage};
use crate::server::entry::Entry;

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
    /// `intent` is the model's one-line statement of purpose. `text` is the
    /// assistant text that accompanied the call (usually empty — a pure
    /// tool round has none); `first` marks the message's opening call so a
    /// multi-call message replays as one Assistant message, not N.
    ToolStart {
        call_id: String,
        name: String,
        args: String,
        intent: String,
        text: String,
        first: bool,
    },
    /// A running tool reported something (see `loop_rs::ToolProgress`).
    ///
    /// Unlike the reply deltas this is **not** reply text: it is the tool's own
    /// narration of what it is doing right now (a command's output so far, a
    /// wait's reason). It reaches the live row / the pending card, never the
    /// transcript, and it is never sent to the model.
    ToolProgress {
        call_id: String,
        chunk: String,
    },
    /// The `todo` tool changed the list. The session records it as an
    /// [`Entry::Todo`] — the state the model's own memo lives in, which is why
    /// it is persisted rather than derived from the tool result.
    Todo { phases: Vec<crate::server::entry::TodoPhase> },
    /// A tool finished executing.
    ///
    /// `result` is the model-facing text. `details` is the tool's structured
    /// payload for front ends — the data a renderer draws from instead of
    /// re-parsing prose (see `loop_rs::ToolOutput`); `None` when the tool had
    /// nothing structured to say. `duration_ms` is measured by the loop, so
    /// it is present for every tool whether or not the tool tracks time.
    ToolFinish {
        call_id: String,
        name: String,
        ok: bool,
        result: String,
        details: Option<serde_json::Value>,
        duration_ms: u64,
    },
    /// An error occurred. If a turn is streaming, this is a mid-flight
    /// death (network drop, malformed stream): whatever the turn produced
    /// is finalized and persisted before the error is recorded — a
    /// conversation must stay replayable even when the connection dies.
    Error(String),
    /// A turn finished: usage for billing, stop_reason to distinguish
    /// interrupts. Finalizes the streaming slots into the reply entry and
    /// **persists the whole round** (the session's `pending` buffer is the
    /// single source of truth — there is no separate assemble-and-commit
    /// step, so the stored round is exactly what streamed).
    TurnDone(Usage, StopReason),
    /// The turn runner thread is exiting.
    Done,

    // ---- user input (from any surface) ----
    /// The user submitted a message; starts a turn (or is rejected while
    /// one streams — the emitter checks `busy()` first, not the session).
    Submit(String),
    /// /name: rename the session. Persists a Name marker under the
    /// current leaf (branches inherit names, siblings never see them).
    NameMarker(String),
    /// /cd landed: persist the migrated working directory. The block it is
    /// recorded against is the session's current tip, which storage owns — the
    /// caller no longer tracks a cursor of its own.
    /// Pure bookkeeping — no transcript change.
    SetCwd { path: String },
    /// The request header of the round that is about to start: everything the
    /// gateway will see that is **not** part of the transcript — model id,
    /// protocol, endpoint, system prompt, tool manuals, token ceiling.
    ///
    /// Sent by the turn runner *before* the first request of the round, so a
    /// round that dies mid-flight still has a header to be stored with its
    /// partial reply. The session parks it and writes it inside the same
    /// transaction as the round's entries.
    RequestMeta {
        model: String,
        protocol: String,
        base_url: String,
        system: String,
        tools_json: String,
        max_tokens: u32,
    },
    /// Compaction finished on the background thread: apply the fork.
    /// `entries` is the marker to persist; `ctx` replaces the live chat
    /// replica; the token stats are display-only.
    Compaction {
        entries: Vec<Entry>,
        ctx: crate::server::ai::types::Context,
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
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
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
#[derive(Debug, Default, Clone, PartialEq)]
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
    /// Output the **running** tool has produced so far (see
    /// [`SessionEvent::ToolProgress`]). Bounded to the tail: while a command
    /// runs the interesting part is what it just printed, and the final result
    /// carries the whole thing anyway.
    pub tool_output: String,
}

impl StreamView {
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
            && self.reasoning.is_empty()
            && self.tool_output.is_empty()
            && !self.active
    }

    /// Fold the streaming slots into one coarse state (see [`RunState`]).
    pub fn run_state(&self) -> RunState {
        if !self.active {
            return RunState::Idle;
        }
        match &self.live {
            LiveActivity::Tool { intent } => RunState::Tool {
                intent: intent.clone(),
            },
            LiveActivity::Idle | LiveActivity::Thinking => {
                if self.text.is_empty() {
                    RunState::Thinking
                } else {
                    RunState::Replying
                }
            }
        }
    }
}

/// The coarse "what is the backend doing right now" — one value, no payload
/// parsing.
///
/// For clients that cannot render a transcript (a chat bridge, a status LED,
/// a web header): they need to say "思考中" / "正在调用工具" without decoding
/// deltas or tool arguments. Derived from the streaming slots, so it costs
/// nothing and cannot drift from what the rich clients show.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// Nothing in flight.
    Idle,
    /// A request is in flight or reasoning is arriving — nothing to show yet
    /// but "it is working".
    Thinking,
    /// Reply text is arriving.
    Replying,
    /// A tool is executing. `intent` is the model's own one-liner (empty when
    /// it offered none).
    Tool { intent: String },
}
