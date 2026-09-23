//! Session state — the **server-side** conversation model, free of TUI
//! dependencies (no ratatui, no event enums). The TUI subscribes to it:
//! it mutates via the narrow mutators below and renders from
//! `transcript()` / `snapshot()`.
//!
//! Owns the "session viscera" that used to sit directly on `App`:
//! transcript, pending round entries, store handle, session id, name,
//! cwd sequence. Cross-cutting flows — command echoes, turn commits,
//! tree navigation, /name markers — all go through here, so the same
//! rules apply no matter which surface triggered them.

use crate::ai::types::Usage;
use crate::entry::Entry;
use crate::server::events::{Change, SessionEvent, StreamView};
use crate::store::Store;

/// Everything the renderer needs from the session, in one read-only
/// snapshot (the TUI never mutates through this).
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub transcript: Vec<Entry>,
    pub session_name: Option<String>,
    pub session_id: Option<i64>,
}


pub struct SessionState {
    // Rendered entries (in-memory + DB-resumed share one path).
    transcript: Vec<Entry>,
    // Entries produced this round; verified/persisted at TurnDone (Commit).
    pending: Vec<Entry>,
    // Storage. None = DB unavailable (degrades to in-memory session).
    store: Option<Store>,
    // Current session id; None until the first turn (session created lazily).
    session_id: Option<i64>,
    // Name set explicitly via /name; None = statusline synthesizes one.
    session_name: Option<String>,
    // Sequence for persisted migrations (cwd_history.seq; 0 = origin).
    cwd_seq: i64,
    // ---- streaming slots (owned here; the TUI only reads StreamView) ----
    stream: StreamView,
    // Last turn's usage (TurnDone), consumed by the caller's cost tracker.
    last_usage: Option<Usage>,
}

impl SessionState {
    pub fn new(store: Option<Store>) -> Self {
        Self {
            transcript: Vec::new(),
            pending: Vec::new(),
            store,
            session_id: None,
            session_name: None,
            cwd_seq: 0,
            stream: StreamView::default(),
            last_usage: None,
        }
    }

    // ---- event intake (the protocolized write side) ----

    /// Consume one protocol event, in order. This is the *only* path a
    /// turn's data takes into the session; input surfaces and the turn
    /// runner both speak [`SessionEvent`]. Returns what changed so the
    /// renderer reacts without learning how.
    ///
    /// Usage lands on `last_usage` (read via [`take_last_usage`]); the
    /// caller owns the model's price sheet and does the local pricing —
    /// the session stays pricing-agnostic.
    pub fn handle(&mut self, ev: SessionEvent) -> Change {
        match ev {
            SessionEvent::Delta(d) => {
                self.stream.reasoning_done = true; // content started; reasoning frozen
                self.stream.text.push_str(&d);
                Change::Stream
            }
            SessionEvent::ReasoningDelta(r) => {
                self.stream.reasoning.push_str(&r);
                Change::Stream
            }
            SessionEvent::ToolStart { call_id, name, args_summary } => {
                let e = Entry::ToolRequest { call_id, name, object: args_summary };
                self.pending.push(e.clone());
                self.transcript.push(e);
                Change::Transcript
            }
            SessionEvent::ToolFinish { call_id, name, ok, result } => {
                let e = Entry::ToolResult {
                    call_id,
                    name,
                    ok,
                    // Store data only (the raw text); the view is
                    // synthesized at render time
                    result,
                };
                self.pending.push(e.clone());
                self.transcript.push(e);
                Change::Transcript
            }
            SessionEvent::Error(e) => {
                // Session-level errors are not persisted (not one of the
                // four message kinds); memory stream only
                self.transcript.push(Entry::Error { text: e });
                Change::Transcript
            }
            SessionEvent::TurnDone(u, _stop) => {
                let content = std::mem::take(&mut self.stream.text);
                let content = if content.is_empty() { "(无输出)".into() } else { content };
                let e = Entry::Assistant {
                    content,
                    usage: Some(Entry::usage_summary(&u)),
                    reasoning: if self.stream.reasoning.is_empty() {
                        None
                    } else {
                        Some(self.stream.reasoning.clone())
                    },
                };
                self.pending.push(e.clone());
                self.transcript.push(e);
                self.last_usage = Some(u);
                self.stream.active = false;
                Change::TurnDone
            }
            SessionEvent::Commit(entries) => {
                // Persist the finalized round in one shot. `entries` is
                // the runner's authoritative assembly (trusted over our
                // incremental pending mirror, which also holds TurnDone's
                // Assistant entry that the runner's list lacks).
                if let Some((st, sid)) = self.persistence()
                    && let Err(e) = st.append(sid, &entries)
                {
                    let msg = format!("落盘失败：{e:#}");
                    self.transcript.push(Entry::Error { text: msg });
                }
                self.pending.clear();
                Change::Transcript
            }
            SessionEvent::Done => {
                self.stream.active = false;
                Change::Stream
            }
            SessionEvent::Submit(_) => {
                // Turn *starting* is the surface's job (it owns the turn
                // runner and the Client); the session only records the
                // resulting entries. Nothing to consume here.
                Change::None
            }
        }
    }

    /// The last turn's usage (set by TurnDone). Cleared on read; the
    /// caller folds it into its cost tracker.
    pub fn take_last_usage(&mut self) -> Option<Usage> {
        self.last_usage.take()
    }

    /// Read-only view of the streaming slots (renderer subscription).
    pub fn stream_view(&self) -> &StreamView {
        &self.stream
    }

    /// Is a turn currently streaming? (Surfaces check this before
    /// starting a new one.)
    pub fn busy(&self) -> bool {
        self.stream.active
    }

    /// Start a turn: echo + stage the user message and reset the
    /// streaming slots. Called by the surface right before spawning the
    /// turn runner — one protocol action instead of three field pokes.
    pub fn start_turn(&mut self, user_text: &str) -> Change {
        let e = Entry::User { content: user_text.to_string() };
        self.transcript.push(e.clone());
        self.pending.push(e);
        self.stream = StreamView { active: true, ..Default::default() };
        Change::Stream
    }

    // ---- read side (renderer subscription) ----

    pub fn transcript(&self) -> &[Entry] {
        &self.transcript
    }

    pub fn session_name(&self) -> Option<&str> {
        self.session_name.as_deref()
    }

    pub fn session_id(&self) -> Option<i64> {
        self.session_id
    }

    pub fn cwd_seq(&self) -> i64 {
        self.cwd_seq
    }

    pub fn store(&self) -> Option<&Store> {
        self.store.as_ref()
    }

    pub fn store_mut(&mut self) -> Option<&mut Store> {
        self.store.as_mut()
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            transcript: self.transcript.clone(),
            session_name: self.session_name.clone(),
            session_id: self.session_id,
        }
    }

    pub fn pending_snapshot(&self) -> &[Entry] {
        &self.pending
    }

    // ---- write side (narrow mutators) ----

    /// Append an echo/status entry to the transcript (never persisted).
    pub fn echo(&mut self, e: Entry) -> Change {
        self.transcript.push(e);
        Change::Transcript
    }

    /// Queue a round entry into `pending` (persisted at TurnDone).
    pub fn stage(&mut self, e: Entry) {
        self.pending.push(e);
    }

    /// Queue several round entries.
    pub fn extend_pending(&mut self, es: impl IntoIterator<Item = Entry>) {
        self.pending.extend(es);
    }

    /// Commit a finalized round: replace the transcript with `entries`
    /// (which include the merged pending tail), clear pending, and if a
    /// session exists persist everything not yet in the store.
    pub fn commit_round(&mut self, entries: Vec<Entry>) {
        self.transcript = entries.clone();
        self.pending.clear();
    }

    /// Set the pending entries wholesale (TurnDone verification path).
    pub fn set_pending(&mut self, es: Vec<Entry>) {
        self.pending = es;
    }

    /// Drop the pending entries (they were persisted via Commit).
    pub fn clear_pending(&mut self) {
        self.pending.clear();
    }

    /// Create a fresh session (or adopt an existing one) and adopt the
    /// given entries as the transcript. Returns the session id.
    pub fn adopt_session(
        &mut self,
        id: i64,
        entries: Vec<Entry>,
        name: Option<String>,
    ) -> Change {
        self.session_id = Some(id);
        self.transcript = entries;
        self.session_name = name;
        Change::Session
    }

    /// Record the effective session name (resume path).
    pub fn set_session_name(&mut self, name: Option<String>) {
        self.session_name = name;
    }

    /// /name: remember the name, stage the marker entry (persisted with
    /// the turn) and echo it to the transcript.
    pub fn name_session(&mut self, arg: &str, marker: Entry) -> Change {
        self.session_name = Some(arg.to_string());
        self.pending.push(marker.clone());
        self.transcript.push(marker);
        Change::Transcript
    }

    /// Tree navigation landed: replace transcript + pending wholesale
    /// and merge the effective name (never clobbers an explicit /name).
    pub fn navigate_to(&mut self, entries: Vec<Entry>, effective_name: Option<String>) -> Change {
        self.transcript = entries.clone();
        self.pending.clear();
        self.session_name = effective_name.or(self.session_name.take());
        Change::Session
    }

    /// Bump the cwd migration sequence (after a successful record_cwd).
    pub fn bump_cwd_seq(&mut self) -> i64 {
        self.cwd_seq += 1;
        self.cwd_seq
    }

    /// Set the cwd migration sequence outright (resume restores it from
    /// the persisted history's last seq).
    pub fn set_cwd_seq(&mut self, seq: i64) {
        self.cwd_seq = seq;
    }

    /// Try to create the session lazily on the first turn. Returns the
    /// new id, or None when the store is unavailable (in-memory mode).
    pub fn ensure_session(&mut self, root: &std::path::Path) -> Option<i64> {
        if self.session_id.is_some() {
            return self.session_id;
        }
        let st = self.store.as_mut()?;
        match st.create_session(&crate::store::now_stamp(), &root.display().to_string()) {
            Ok(id) => {
                self.session_id = Some(id);
                Some(id)
            }
            Err(e) => {
                self.transcript.push(Entry::Error { text: format!("会话创建失败：{e:#}") });
                None
            }
        }
    }

    /// Store + id, paired — most persistence flows need both.
    pub fn persistence(&mut self) -> Option<(&mut Store, i64)> {
        let sid = self.session_id?;
        Some((self.store.as_mut()?, sid))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st() -> SessionState {
        SessionState::new(None) // in-memory mode
    }

    #[test]
    fn echo_grows_transcript() {
        let mut s = st();
        assert_eq!(s.echo(Entry::Error { text: "x".into() }), Change::Transcript);
        assert_eq!(s.transcript().len(), 1);
    }

    #[test]
    fn commit_round_clears_pending() {
        let mut s = st();
        s.stage(Entry::User { content: "hi".into() });
        assert_eq!(s.pending_snapshot().len(), 1);
        s.commit_round(vec![Entry::User { content: "hi".into() }]);
        assert_eq!(s.pending_snapshot().len(), 0);
        assert_eq!(s.transcript().len(), 1);
    }

    #[test]
    fn name_session_updates_statusline_and_stages_marker() {
        let mut s = st();
        let marker = Entry::Name { name: "test".into() };
        s.name_session("test", marker);
        assert_eq!(s.session_name(), Some("test"));
        assert_eq!(s.pending_snapshot().len(), 1);
    }

    #[test]
    fn navigate_merges_name_without_clobbering() {
        let mut s = st();
        s.set_session_name(Some("explicit".into()));
        // Navigation brings an effective name; explicit /name wins.
        s.navigate_to(vec![], Some("branch-name".into()));
        assert_eq!(s.session_name(), Some("branch-name"));
        s.navigate_to(vec![], None);
        assert_eq!(s.session_name(), Some("branch-name"));
    }

    #[test]
    fn in_memory_mode_has_no_persistence() {
        let mut s = st();
        assert!(s.persistence().is_none());
        assert!(s.ensure_session(std::path::Path::new("/tmp")).is_none());
    }
}
