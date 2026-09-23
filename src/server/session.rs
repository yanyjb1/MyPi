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

use crate::ai::client::Client;
use crate::ai::config::{Config, ModelEntry};
use crate::ai::types::{Context as ChatContext, Usage};
use std::sync::{Arc, Mutex};
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
            SessionEvent::NameMarker(name) => {
                // Marker entry (tree semantics) + best-effort legacy column.
                self.session_name = Some(name.clone());
                let marker = Entry::Name { name: name.clone() };
                self.transcript.push(marker.clone());
                // Borrow discipline: store writes in scoped blocks; error
                // echoes go into the transcript only after the borrow ends.
                let write_err = self.persistence().and_then(|(st, sid)| {
                    st.append(sid, std::slice::from_ref(&marker)).err()
                });
                if let Some(e) = write_err {
                    self.transcript.push(Entry::Error { text: format!("命名写入失败：{e:#}") });
                }
                if let Some((st, sid)) = self.persistence() {
                    let _ = st.set_session_name(sid, Some(&name));
                }
                Change::Transcript
            }
            SessionEvent::SetCwd { seq, path } => {
                // Bookkeeping only: nothing to draw.
                if let Some((st, sid)) = self.persistence() {
                    let _ = st.record_cwd(sid, seq, &path);
                }
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


// ---- facade ------------------------------------------------------------

/// High-level session handle — the object a surface *holds*.
///
/// Owns everything a turn needs (client, shared chat replica, cwd slot,
/// interrupt flag, price sheet) and exposes protocol-level operations:
/// `submit` spawns the turn runner and routes its events into the
/// [`SessionState`]. A surface (TUI or a headless driver) never touches
/// turn viscera — it calls methods here and drains `Change`s.
pub struct Session {
    pub state: SessionState,
    // Turn resources (shared with the spawned runner thread).
    client: std::cell::RefCell<Client>,
    chat: Arc<Mutex<ChatContext>>,
    pub max_tokens: u32,
    pub interrupt: Arc<std::sync::atomic::AtomicBool>,
    // Working directory (shared; /cd migrates it): the turn thread
    // snapshots at start, the surface writes; the RwLock keeps reads and
    // writes from trampling each other.
    pub cwd: Arc<std::sync::RwLock<std::path::PathBuf>>,
    // Price sheet of the current model (local cost computation).
    pub cost: crate::ai::config::Cost,
    cost_tracker: crate::ai::pricing::CostTracker,
    tx: std::sync::mpsc::Sender<SessionEvent>,
}

impl Session {
    /// Assemble a session service + its event channel. `chat` starts as
    /// the system-prompt-only replica.
    pub fn new(
        state: SessionState,
        client: Client,
        chat: ChatContext,
        max_tokens: u32,
        cost: crate::ai::config::Cost,
        cwd: std::path::PathBuf,
    ) -> (Self, std::sync::mpsc::Receiver<SessionEvent>) {
        let (tx, rx) = std::sync::mpsc::channel::<SessionEvent>();
        let s = Self {
            state,
            client: std::cell::RefCell::new(client),
            chat: Arc::new(Mutex::new(chat)),
            max_tokens,
            interrupt: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            cwd: Arc::new(std::sync::RwLock::new(cwd)),
            cost,
            cost_tracker: crate::ai::pricing::CostTracker::default(),
            tx,
        };
        (s, rx)
    }

    /// Submit user text: lazily create the session (in-memory mode
    /// degrades gracefully), echo + stage + reset streaming slots, then
    /// spawn the turn runner. Returns false when a turn is already
    /// streaming.
    pub fn submit(&mut self, text: &str) -> bool {
        if self.state.busy() {
            return false;
        }
        if self.state.session_id().is_none() {
            let cwd = self.cwd().display().to_string();
            let Some(st) = self.state.store_mut() else {
                return self.finish_submit(text);
            };
            {
            let now = crate::store::now_stamp();
            match st.create_session(&now, &cwd) {
                Ok(id) => {
                    self.state.adopt_session(id, Vec::new(), None);
                }
                Err(e) => {
                    self.state
                        .echo(crate::entry::Entry::Error { text: format!("数据库不可用：{e:#}") });
                }
            }
            }
        }
        self.finish_submit(text)
    }

    fn finish_submit(&mut self, text: &str) -> bool {
        self.state.start_turn(text);
        self.interrupt
            .store(false, std::sync::atomic::Ordering::Relaxed);
        crate::server::turn::spawn_turn(
            self.tx.clone(),
            crate::server::turn::TurnRequest {
                client: self.client.borrow().clone(),
                chat: self.chat.clone(),
                text: text.to_string(),
                cfg: crate::agent::loop_rs::LoopConfig::new(self.max_tokens),
                interrupt: self.interrupt.clone(),
                cwd: self.cwd.read().expect("cwd 锁中毒").clone(),
                cwd_slot: self.cwd.clone(),
            },
        );
        true
    }

    /// Point the client at another model (the /switch path). Returns the
    /// new current model entry.
    pub fn switch_model(&self, cfg: &Config, rm: &crate::ai::config::ResolvedModel) -> ModelEntry {
        let provider = cfg
            .models
            .providers
            .get(&rm.provider_name)
            .expect("validated provider");
        let api_key = cfg.resolve_key(provider);
        self.client
            .borrow_mut()
            .switch_model(&provider.base_url, &api_key, &rm.entry.id);
        rm.entry.clone()
    }

    /// Rebuild the shared chat replica from projected entries (tree
    /// navigation / resume). Dangling tool tails are repaired inside.
    pub fn rebuild_chat(&self, entries: &[Entry]) {
        *self.chat.lock().expect("chat 锁中毒") =
            crate::server::turn::entries_to_context(entries);
    }

    /// Migrate the working directory; returns the previous value.
    pub fn set_cwd(&self, next: std::path::PathBuf) -> std::path::PathBuf {
        let mut w = self.cwd.write().expect("cwd 锁中毒");
        std::mem::replace(&mut *w, next)
    }

    pub fn cwd(&self) -> std::path::PathBuf {
        self.cwd.read().expect("cwd 锁中毒").clone()
    }

    /// Current model's display price: total spent + latest prompt tokens.
    pub fn spend(&self) -> (f64, u64) {
        (self.cost_tracker.total, self.cost_tracker.last_prompt_tokens)
    }

    // ---- read-side passthrough (the surface renders through these) ----

    pub fn busy(&self) -> bool {
        self.state.busy()
    }
    pub fn stream_view(&self) -> &StreamView {
        self.state.stream_view()
    }
    pub fn transcript(&self) -> &[Entry] {
        self.state.transcript()
    }
    pub fn snapshot(&self) -> Snapshot {
        self.state.snapshot()
    }
    pub fn session_name(&self) -> Option<&str> {
        self.state.session_name()
    }
    pub fn session_id(&self) -> Option<i64> {
        self.state.session_id()
    }
    pub fn cwd_seq(&self) -> i64 {
        self.state.cwd_seq()
    }
    pub fn store(&self) -> Option<&Store> {
        self.state.store()
    }
    pub fn store_mut(&mut self) -> Option<&mut Store> {
        self.state.store_mut()
    }
    pub fn pending_snapshot(&self) -> &[Entry] {
        self.state.pending_snapshot()
    }

    // ---- write-side passthrough (UI-flow mutators that predate the
    // protocol: echoes, navigation, resume adoption). The turn's data
    // path itself goes through `handle` only. ----

    pub fn echo(&mut self, e: Entry) -> Change {
        self.state.echo(e)
    }
    pub fn stage(&mut self, e: Entry) {
        self.state.stage(e)
    }
    pub fn commit_round(&mut self, entries: Vec<Entry>) {
        self.state.commit_round(entries)
    }
    pub fn adopt_session(&mut self, id: i64, entries: Vec<Entry>, name: Option<String>) -> Change {
        self.state.adopt_session(id, entries, name)
    }
    pub fn set_session_name(&mut self, name: Option<String>) {
        self.state.set_session_name(name)
    }
    pub fn navigate_to(&mut self, entries: Vec<Entry>, effective_name: Option<String>) -> Change {
        self.state.navigate_to(entries, effective_name)
    }
    pub fn bump_cwd_seq(&mut self) -> i64 {
        self.state.bump_cwd_seq()
    }
    pub fn set_cwd_seq(&mut self, seq: i64) {
        self.state.set_cwd_seq(seq)
    }
    pub fn ensure_session(&mut self, root: &std::path::Path) -> Option<i64> {
        self.state.ensure_session(root)
    }
    pub fn persistence(&mut self) -> Option<(&mut Store, i64)> {
        self.state.persistence()
    }
    pub fn handle(&mut self, ev: SessionEvent) -> Change {
        self.state.handle(ev)
    }

    /// Esc during a turn: the runner stops reading and disconnects.
    pub fn interrupt_turn(&self) {
        self.interrupt
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    /// Drain protocol events into the session state. `Change::TurnDone`
    /// is priced locally against `cost` here. Returns the observed
    /// changes so the surface reacts without touching internals.
    pub fn drain(&mut self, rx: &std::sync::mpsc::Receiver<SessionEvent>) -> Vec<Change> {
        let mut changes = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            let change = self.state.handle(ev);
            if change == Change::TurnDone
                && let Some(u) = self.state.take_last_usage()
            {
                self.cost_tracker.record(&u, &self.cost);
            }
            changes.push(change);
        }
        changes
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
    fn name_marker_updates_statusline_and_appends_marker() {
        let mut s = st();
        s.handle(SessionEvent::NameMarker("test".into()));
        assert_eq!(s.session_name(), Some("test"));
        // Marker echoes into the transcript (renderer skips it).
        assert!(matches!(s.transcript().last(), Some(Entry::Name { name }) if name == "test"));
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
