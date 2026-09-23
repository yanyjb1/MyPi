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

use crate::entry::Entry;
use crate::store::Store;

/// Everything the renderer needs from the session, in one read-only
/// snapshot (the TUI never mutates through this).
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub transcript: Vec<Entry>,
    pub session_name: Option<String>,
    pub session_id: Option<i64>,
}

/// What happened after a mutation — the TUI reacts (redraw, scroll pin)
/// without learning *how* the state changed internally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// Transcript contents changed (echo, commit, navigation...).
    Transcript,
    /// The current session changed (new session created, resumed...).
    Session,
    /// Nothing observable happened (e.g. store unavailable, no-op).
    None,
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
        }
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
