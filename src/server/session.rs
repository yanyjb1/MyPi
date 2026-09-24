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
use crate::entry::Entry;
use crate::server::events::{Change, LiveActivity, SessionEvent, StreamView};
use crate::store::Store;
use std::sync::{Arc, Mutex};

pub struct SessionState {
    // Rendered entries (in-memory + DB-resumed share one path).
    transcript: Vec<Entry>,
    // Bumped whenever the transcript is **replaced** wholesale (tree
    // navigation, resume) rather than appended to. The render cache keys
    // on this: a swap to a same-length branch is invisible to a length
    // check, so the rows of the branch you left would keep painting.
    transcript_generation: u64,
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
    // Last turn's usage snapshot, parked for `Commit` to fold into the reply
    // entry the runner assembles (which carries no usage of its own).
    pending_usage: Option<Usage>,
}

impl SessionState {
    pub fn new(store: Option<Store>) -> Self {
        Self {
            transcript: Vec::new(),
            transcript_generation: 0,
            pending: Vec::new(),
            store,
            session_id: None,
            session_name: None,
            cwd_seq: 0,
            stream: StreamView::default(),
            last_usage: None,
            pending_usage: None,
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
                // Content is arriving: it occupies the live row itself.
                self.stream.live = LiveActivity::Idle;
                Change::Stream
            }
            SessionEvent::ReasoningDelta(r) => {
                // First reasoning chunk of this turn means the server has
                // started talking — and it is talking about its own thoughts
                // rather than answering. That is exactly "thinking".
                if self.stream.reasoning.is_empty() {
                    self.stream.live = LiveActivity::Thinking;
                }
                self.stream.reasoning.push_str(&r);
                Change::Stream
            }
            SessionEvent::ToolStart {
                call_id,
                name,
                args,
                intent,
                text,
                first,
            } => {
                self.stream.live = LiveActivity::Tool {
                    intent: intent.clone(),
                };
                // The message's text and its opening call are captured here;
                // the round's own text buffer is cleared because it belongs
                // to this tool message now, not to the final reply (else the
                // two would merge into one reply on screen and on disk).
                if first && !text.is_empty() {
                    self.stream.text.clear();
                }
                let e = Entry::ToolRequest {
                    call_id,
                    name,
                    args,
                    intent,
                    text,
                    first,
                };
                self.pending.push(e.clone());
                self.transcript.push(e);
                Change::Transcript
            }
            SessionEvent::ToolFinish {
                call_id,
                name,
                ok,
                result,
            } => {
                // The call this row described has landed; the next event
                // (another tool, or the reply) decides what replaces it.
                self.stream.live = LiveActivity::Idle;
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
                Change::ToolActivity
            }
            SessionEvent::Error(e) => {
                // Mid-flight death (network drop, malformed stream) or a
                // session-level notice. If a turn was streaming, whatever it
                // produced is finalized **and persisted** first: a dropped
                // connection must not cost the user the partial reply they
                // already watched arrive. The error itself is display-only
                // (not a protocol message kind).
                if self.stream.active {
                    self.finalize_round(None);
                }
                self.transcript.push(Entry::Error { text: e });
                Change::Transcript
            }
            SessionEvent::TurnDone(u, _stop) => {
                // The single assembly point: whatever the event stream built
                // up in `pending` IS the round. No second assembler runs, so
                // there is nothing to keep in sync and no lossy projection
                // between what streamed and what is stored.
                self.finalize_round(Some(u.clone()));
                self.pending_usage = Some(u.clone());
                self.last_usage = Some(u);
                Change::TurnDone
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
                let write_err = self
                    .persistence()
                    .and_then(|(st, sid)| st.append(sid, std::slice::from_ref(&marker)).err());
                if let Some(e) = write_err {
                    self.transcript.push(Entry::Error {
                        text: format!("命名写入失败：{e:#}"),
                    });
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
            SessionEvent::Compaction { .. } => {
                // Intercepted by `Session::ingest` before reaching here —
                // the facade owns the chat replica this event replaces.
                Change::None
            }
        }
    }

    /// The **single** assembly point for a round. Folds the streaming
    /// buffers into entries (reasoning first, then the reply riding usage),
    /// then persists everything `pending` accumulated this round and clears
    /// it. Called at TurnDone *and* on a mid-flight Error, so a dropped
    /// connection stores exactly what streamed instead of losing the round.
    ///
    /// `usage` is `None` on the error path (the stream died before the
    /// final usage chunk arrived); the reply entry is still written, just
    /// without a stats line.
    fn finalize_round(&mut self, usage: Option<Usage>) {
        let content = std::mem::take(&mut self.stream.text);
        let content = if content.is_empty() {
            "(无输出)".into()
        } else {
            content
        };
        let reasoning = std::mem::take(&mut self.stream.reasoning);
        if !reasoning.is_empty() {
            let r = Entry::Reasoning { content: reasoning };
            self.pending.push(r.clone());
            self.transcript.push(r);
        }
        let e = Entry::Assistant {
            content,
            usage: usage.map(|u| Entry::usage_summary(&u)),
        };
        self.pending.push(e.clone());
        self.transcript.push(e);
        self.stream.active = false;

        // Take the round out before persisting: `persistence()` borrows
        // `self` mutably and the store write must not alias the buffer.
        let round = std::mem::take(&mut self.pending);
        if let Some((st, sid)) = self.persistence()
            && let Err(e) = st.append(sid, &round)
        {
            let msg = format!("落盘失败：{e:#}");
            self.transcript.push(Entry::Error { text: msg });
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
        let e = Entry::User {
            content: user_text.to_string(),
        };
        self.transcript.push(e.clone());
        self.pending.push(e);
        self.stream = StreamView {
            active: true,
            ..Default::default()
        };
        Change::Stream
    }

    // ---- read side (renderer subscription) ----

    pub fn transcript(&self) -> &[Entry] {
        &self.transcript
    }

    /// Generation of the current transcript (0 = never replaced). The render
    /// cache compares this across frames to notice a wholesale swap.
    pub fn transcript_generation(&self) -> u64 {
        self.transcript_generation
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
        self.replace_transcript(entries);
        self.pending.clear();
    }

    /// Swap in a whole new transcript and bump the generation so the render
    /// cache drops the previous one. `replace_transcript` is the *only*
    /// path that assigns `transcript` — one place to forget the bump.
    fn replace_transcript(&mut self, entries: Vec<Entry>) {
        self.transcript = entries;
        self.transcript_generation = self.transcript_generation.wrapping_add(1);
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
    pub fn adopt_session(&mut self, id: i64, entries: Vec<Entry>, name: Option<String>) -> Change {
        self.session_id = Some(id);
        self.replace_transcript(entries);
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
        self.replace_transcript(entries);
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
                self.transcript.push(Entry::Error {
                    text: format!("会话创建失败：{e:#}"),
                });
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
    // Profile tool roster (see `TurnRequest.tool_filter`). Set at
    // startup from the active profile; the profile switch command only
    // re-reads it at the next rebuild point.
    tool_filter: Option<Vec<String>>,
    // Tool-layer knobs from config.yaml (`tools:`). Snapshotted per turn;
    // a config edit takes effect on the next turn.
    tools: crate::agent::tools::ToolsConfig,
    // Session DB path (file-backed store): the turn thread opens its
    // own connection from here to spill/fetch artifacts.
    artifact_db: Option<std::path::PathBuf>,
}

impl Session {
    /// Assemble a session service + its event channel. `chat` starts as
    /// the system-prompt-only replica.
    // A constructor wiring session resources one-to-one; a params struct
    // would just mirror these fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: SessionState,
        client: Client,
        chat: ChatContext,
        max_tokens: u32,
        cost: crate::ai::config::Cost,
        cwd: std::path::PathBuf,
        tool_filter: Option<Vec<String>>,
        tools: crate::agent::tools::ToolsConfig,
        db_path: Option<std::path::PathBuf>,
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
            tool_filter,
            tools,
            artifact_db: db_path,
        };
        (s, rx)
    }

    /// `/compact [focus]`: run context compaction on a background thread.
    ///
    /// The summarization round-trip is a blocking network call (the
    /// non-streaming `complete` — no per-character display needed), so it
    /// must not sit on the UI thread. The thread owns a cloned `Client`
    /// (the real one stays behind the RefCell); results come back through
    /// the event channel:
    ///
    /// - `Error` echoes the failure (context untouched; safe to retry),
    /// - `Compaction { entries, ctx }` lets the session apply the fork
    ///   atomically: persist the marker entry, swap the live context,
    ///   echo the divider.
    ///
    /// Returns false when a turn is already streaming (compaction shares
    /// the busy gate — two writers on one context is a torn read).
    pub fn run_compact(
        &mut self,
        focus: &str,
        ccfg: &crate::server::compaction::CompactConfig,
    ) -> bool {
        if self.state.busy() {
            return false;
        }
        let client = self.client.borrow().clone();
        let chat = self.chat.clone();
        let tx = self.tx.clone();
        let cwd_snapshot = self.state.transcript().to_vec();
        let system = chat
            .lock()
            .expect("chat 锁中毒")
            .messages
            .first()
            .map(|m| match m {
                crate::ai::types::Message::System { content } => content.clone(),
                _ => String::new(),
            })
            .unwrap_or_default();
        let ccfg = ccfg.clone();
        let focus = focus.to_string();
        let max_tokens = self.max_tokens;
        std::thread::spawn(move || {
            let mut summarize = |req: &crate::ai::types::Context| -> anyhow::Result<String> {
                let reply = client.complete(req, max_tokens)?;
                Ok(reply.content)
            };
            match crate::server::compaction::compact(
                &system,
                &cwd_snapshot,
                &ccfg,
                &focus,
                &mut summarize,
            ) {
                Ok(out) => {
                    let _ = tx.send(SessionEvent::Compaction {
                        entries: vec![out.marker],
                        ctx: out.ctx,
                        tokens_before: out.tokens_before,
                        tokens_after: out.tokens_after,
                    });
                }
                Err(e) => {
                    let _ = tx.send(SessionEvent::Error(format!("{e:#}")));
                }
            }
        });
        true
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
                        self.state.echo(crate::entry::Entry::Error {
                            text: format!("数据库不可用：{e:#}"),
                        });
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
                history: std::sync::Arc::new(self.state.transcript().to_vec()),
                cwd_trail: std::sync::Arc::new(
                    self.state
                        .persistence()
                        .and_then(|(st, sid)| st.cwd_history(sid).ok())
                        .unwrap_or_default(),
                ),
                tool_filter: self.tool_filter.clone(),
                tools: self.tools.clone(),
                // Session-scoped artifact store: the turn thread gets its
                // own WAL-mode connection; a failed open degrades to None
                // (oversized output then stays verbatim in the context).
                artifacts: match (self.artifact_db.as_deref(), self.state.session_id()) {
                    (Some(path), Some(sid)) => {
                        crate::agent::artifacts::ArtifactStore::open(path, sid)
                    }
                    _ => None,
                },
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
    /// navigation / resume). Dangling tool tails are repaired inside, and
    /// the **current** system prompt (the live profile's) is preserved —
    /// resume must not silently swap the prompt back to the built-in one.
    pub fn rebuild_chat(&self, entries: &[Entry]) {
        let system = {
            let chat = self.chat.lock().expect("chat 锁中毒");
            match chat.messages.first() {
                Some(crate::ai::types::Message::System { content }) => content.clone(),
                _ => crate::server::profile::BUILTIN_SYSTEM.to_string(),
            }
        };
        *self.chat.lock().expect("chat 锁中毒") =
            crate::server::turn::entries_to_context(&system, entries);
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
        (
            self.cost_tracker.total,
            self.cost_tracker.last_prompt_tokens,
        )
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
    pub fn transcript_generation(&self) -> u64 {
        self.state.transcript_generation()
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
    pub fn ingest(&mut self, ev: SessionEvent) -> Change {
        // Compaction lands at the facade: it must touch the chat replica
        // (owned here, not by SessionState) and the store in one go.
        if let SessionEvent::Compaction {
            entries,
            ctx,
            tokens_before,
            tokens_after,
        } = ev
        {
            return self.apply_compaction(entries, ctx, tokens_before, tokens_after);
        }
        let change = self.state.handle(ev);
        if change == Change::TurnDone
            && let Some(u) = self.state.take_last_usage()
        {
            self.cost_tracker.record(&u, &self.cost);
        }
        change
    }

    /// Atomically apply a finished compaction: persist the fork marker,
    /// swap the live context, echo the divider + stats.
    fn apply_compaction(
        &mut self,
        entries: Vec<Entry>,
        ctx: ChatContext,
        tokens_before: usize,
        tokens_after: usize,
    ) -> Change {
        // 1) Persist the marker under the current leaf (the tree keeps
        //    the pre-compact branch reachable).
        if let Some((st, sid)) = self.state.persistence()
            && let Err(e) = st.append(sid, &entries)
        {
            let msg = format!("压缩标记落盘失败：{e:#}");
            self.state.echo(Entry::Error { text: msg });
        }
        // 2) Swap the live context replica: next turn starts from
        //    system + summary turn + kept region (prefix-cache cold
        //    once, then warm).
        *self.chat.lock().expect("chat 锁中毒") = ctx;
        // 3) Transcript: the divider marker + the display stats.
        for e in entries {
            self.state.echo(e);
        }
        self.state.echo(Entry::System {
            text: format!("上下文已压缩：≈{tokens_before} → ≈{tokens_after} tokens"),
            align: crate::entry::Align::Center,
        });
        Change::Session
    }

    pub fn drain(&mut self, rx: &std::sync::mpsc::Receiver<SessionEvent>) -> Vec<Change> {
        let mut changes = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            changes.push(self.ingest(ev));
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
    fn a_plain_text_round_persists_what_streamed() {
        // Drive the real event flow end to end: user submit, deltas, done.
        // The round that lands in the store must be exactly the events the
        // session consumed — no second assembler exists to disagree.
        let dir = std::env::temp_dir().join(format!("mypi-plain-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = SessionState::new(Some(Store::open(&dir.join("t.db")).unwrap()));
        let id = s.ensure_session(&dir).unwrap();
        let _ = s.start_turn("问");
        let _ = s.handle(SessionEvent::Delta("答案".into()));
        let _ = s.handle(SessionEvent::TurnDone(
            Usage::default(),
            crate::ai::types::StopReason::Stop,
        ));
        let back = s.store().unwrap().load_entries(id).unwrap();
        assert_eq!(
            back,
            vec![
                Entry::User {
                    content: "问".into()
                },
                Entry::Assistant {
                    content: "答案".into(),
                    usage: Some(Entry::usage_summary(&Usage::default())),
                },
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tool_round_persists_requests_results_and_reply_with_reasoning() {
        // A tool round driven through events: the reasoning buffer folds in
        // right before the reply, the tool pair keeps its real `ok`, and the
        // round lands in one transaction.
        let dir = std::env::temp_dir().join(format!("mypi-toolround-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = SessionState::new(Some(Store::open(&dir.join("t.db")).unwrap()));
        let id = s.ensure_session(&dir).unwrap();
        let _ = s.start_turn("跑个命令");
        let _ = s.handle(SessionEvent::ToolStart {
            call_id: "c1".into(),
            name: "bash".into(),
            args: "{}".into(),
            intent: "跑".into(),
            text: String::new(),
            first: true,
        });
        let _ = s.handle(SessionEvent::ToolFinish {
            call_id: "c1".into(),
            name: "bash".into(),
            ok: false,
            result: "boom".into(),
        });
        let _ = s.handle(SessionEvent::ReasoningDelta("查一下".into()));
        let _ = s.handle(SessionEvent::Delta("搞定".into()));
        let _ = s.handle(SessionEvent::TurnDone(
            Usage::default(),
            crate::ai::types::StopReason::Stop,
        ));
        let back = s.store().unwrap().load_entries(id).unwrap();
        let kinds: Vec<&str> = back
            .iter()
            .map(|e| match e {
                Entry::User { .. } => "user",
                Entry::Reasoning { .. } => "reasoning",
                Entry::Assistant { .. } => "assistant",
                Entry::ToolRequest { .. } => "tool_request",
                Entry::ToolResult { .. } => "tool_result",
                _ => "other",
            })
            .collect();
        assert_eq!(
            kinds,
            vec![
                "user",
                "tool_request",
                "tool_result",
                "reasoning",
                "assistant"
            ],
            "思考必须紧邻最终回复、在工具条目之后"
        );
        // The **real** ok survives: a failed tool stays failed on disk.
        let ok = back.iter().find_map(|e| match e {
            Entry::ToolResult { ok, .. } => Some(*ok),
            _ => None,
        });
        assert_eq!(ok, Some(false), "失败的 ok 必须落盘（旧路径硬编码 true）");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reasoning_and_usage_land_on_the_persisted_round() {
        let dir = std::env::temp_dir().join(format!("mypi-commit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = SessionState::new(Some(Store::open(&dir.join("t.db")).unwrap()));
        let id = s.ensure_session(&dir).unwrap();
        let _ = s.start_turn("问");
        let _ = s.handle(SessionEvent::ReasoningDelta("想了想".into()));
        let _ = s.handle(SessionEvent::Delta("答案".into()));
        let _ = s.handle(SessionEvent::TurnDone(
            Usage::default(),
            crate::ai::types::StopReason::Stop,
        ));
        let back = s.store().unwrap().load_entries(id).unwrap();
        let kinds: Vec<&str> = back
            .iter()
            .map(|e| match e {
                Entry::User { .. } => "user",
                Entry::Reasoning { .. } => "reasoning",
                Entry::Assistant { .. } => "assistant",
                _ => "other",
            })
            .collect();
        assert_eq!(kinds, vec!["user", "reasoning", "assistant"]);
        // Usage rode on the reply (the wire Message cannot carry it).
        let usage = back
            .iter()
            .find_map(|e| match e {
                Entry::Assistant { usage, .. } => Some(*usage),
                _ => None,
            })
            .unwrap();
        assert!(usage.is_some(), "usage 必须写在回复条目上");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reasoning_survives_into_the_assistant_entry() {
        let mut s = st();
        let _ = s.handle(SessionEvent::ReasoningDelta("先想".into()));
        let _ = s.handle(SessionEvent::ReasoningDelta("再想".into()));
        let _ = s.handle(SessionEvent::Delta("答案".into()));
        let _ = s.handle(SessionEvent::TurnDone(
            Usage::default(),
            crate::ai::types::StopReason::Stop,
        ));
        let ts = s.transcript();
        let reasoning = ts.iter().find_map(|e| match e {
            Entry::Reasoning { content } => Some(content.clone()),
            _ => None,
        });
        let reply = ts
            .iter()
            .find_map(|e| match e {
                Entry::Assistant { content, .. } => Some(content.clone()),
                _ => None,
            })
            .expect("必须有一条 Assistant");
        assert_eq!(reply, "答案");
        assert_eq!(
            reasoning.as_deref(),
            Some("先想再想"),
            "reasoning 必须落成独立条目"
        );
    }

    #[test]
    fn a_stream_death_still_persists_the_partial_reply() {
        // The hard constraint: a dropped connection must not cost the user
        // the reply they already watched arrive. Error while streaming ->
        // whatever was produced is finalized and stored, then the error is
        // recorded (display-only). Replayable from any break point.
        let dir = std::env::temp_dir().join(format!("mypi-drop-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = SessionState::new(Some(Store::open(&dir.join("t.db")).unwrap()));
        let id = s.ensure_session(&dir).unwrap();
        let _ = s.start_turn("问");
        let _ = s.handle(SessionEvent::Delta("半句".into()));
        let _ = s.handle(SessionEvent::ReasoningDelta("想着".into()));
        // Connection dies mid-stream.
        let _ = s.handle(SessionEvent::Error("stream read interrupted".into()));

        let back = s.store().unwrap().load_entries(id).unwrap();
        let kinds: Vec<&str> = back
            .iter()
            .map(|e| match e {
                Entry::User { .. } => "user",
                Entry::Reasoning { .. } => "reasoning",
                Entry::Assistant { .. } => "assistant",
                _ => "other",
            })
            .collect();
        assert_eq!(
            kinds,
            vec!["user", "reasoning", "assistant"],
            "断网也要落盘已到达的部分"
        );
        let reply = back.iter().find_map(|e| match e {
            Entry::Assistant { content, .. } => Some(content.clone()),
            _ => None,
        });
        assert_eq!(reply.as_deref(), Some("半句"), "部分回复必须保留");
        // The error itself is display-only — not one of the stored kinds.
        assert!(
            back.iter().all(|e| !matches!(e, Entry::Error { .. })),
            "错误提示不落盘（无协议角色）"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tool_finish_signals_tool_activity() {
        // The git-refresh contract: ToolFinish is the checkpoint surfaces
        // refresh the environment on — NOT plain transcript changes.
        let mut s = st();
        let ch = s.handle(SessionEvent::ToolFinish {
            call_id: "c1".into(),
            name: "bash".into(),
            ok: true,
            result: "ok".into(),
        });
        assert_eq!(ch, Change::ToolActivity);
        // Speech-side changes stay Transcript.
        assert_eq!(
            s.handle(SessionEvent::Error("x".into())),
            Change::Transcript
        );
    }

    #[test]
    fn echo_grows_transcript() {
        let mut s = st();
        assert_eq!(
            s.echo(Entry::Error { text: "x".into() }),
            Change::Transcript
        );
        assert_eq!(s.transcript().len(), 1);
    }

    #[test]
    fn commit_round_clears_pending() {
        let mut s = st();
        s.stage(Entry::User {
            content: "hi".into(),
        });
        assert_eq!(s.pending_snapshot().len(), 1);
        s.commit_round(vec![Entry::User {
            content: "hi".into(),
        }]);
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
