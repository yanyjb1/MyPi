//! Session storage: SQLite (WAL).
//!
//! Two tables: `sessions` (metadata) and `entries` (messages, primary key
//! `(session_id, seq)` with `seq` monotonically increasing from 1 within
//! a session).
//!
//! Persistence policy: **write as soon as a message is final**. While
//! streaming, text lives only in the in-memory "in progress" slot and
//! never touches the DB; at TurnDone the whole round's entries are
//! written in one shot. Nothing to do at exit — there is no
//! "finalized but not persisted" state. `journal_mode=WAL` +
//! `synchronous=NORMAL`: writes never block reads, a crash loses only
//! the last round.
//!
//! All writes happen on the TUI main thread (turn threads ship entries
//! back as events), so `Store` needs no locking.

use anyhow::{Context, Result};
use rusqlite::Connection;

// Session metadata.
#[derive(Debug, Clone)]
/// One stored row in flat form (tree picker input).
pub struct TreeNode {
    pub seq: i64,
    pub parent_seq: Option<i64>,
    pub kind: String,
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionMeta {
    pub id: i64,
    // Name set explicitly via /name; NULL = unnamed.
    pub name: Option<String>,
    // Timestamp of the first turn (used when synthesizing a display name).
    pub started_at: String,
    // Latest **persisted** working directory (written by /cdp); NULL =
    // never migrated, resume falls back to the directory at session
    // creation (the caller supplies it).
    pub cwd: Option<String>,
}

/// One resume-picker row: `sessions` metadata plus what the picker draws that
/// the table itself does not carry.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionRow {
    pub meta: SessionMeta,
    /// First user message, verbatim (preview line, and the fallback display
    /// name when the session was never named).
    pub first_user: Option<String>,
    /// Stored bytes of this session's entries — the picker's size column.
    pub bytes: i64,
}

// The part of a request that **cannot** be re-derived from the transcript:
// who we talked to, on which endpoint, under which system prompt, with which
// tool manuals, and how much room the reply had.
//
// Written by the turn runner (the only layer that knows what the request
// actually carried) and persisted with the round it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundMeta {
    pub model: String,
    /// Wire protocol id, e.g. `openai-chat-completions`.
    pub protocol: String,
    /// Endpoint base (`https://host/v1`). Never the API key.
    pub base_url: String,
    /// System prompt **verbatim**, as it entered the request.
    pub system: String,
    /// Tool manuals **verbatim** (the serialized `tools` array).
    pub tools_json: String,
    /// Token ceiling sent with this round's requests (the configured one;
    /// the per-request value the loop derives from it is recomputable).
    pub max_tokens: u32,
}

/// One stored request header (a row of `rounds`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundRow {
    pub seq: i64,
    pub ts: String,
    pub model: String,
    pub protocol: String,
    pub base_url: String,
    pub system: String,
    pub tools_json: String,
    pub max_tokens: u32,
    /// How the round ended; `None` = died before the gateway answered.
    pub stop_reason: Option<String>,
    /// Entry span this round wrote (`None` when the round stored no entries).
    pub first_seq: Option<i64>,
    pub last_seq: Option<i64>,
}

// The projected-path CTE every tree-walking query starts from: the chain the
// session's leaf points at, walked back to the root.
//
// `?1` = session id, `?2` = leaf override (`NULL` = the session's own leaf).
// The recursive step probes the `(session_id, seq)` primary-key index, so the
// walk itself is linear in the path length.
//
// **The outer query must reach the path through `seq IN (SELECT seq FROM path)`,
// never through `JOIN path p ON e.seq = p.seq`.** The join shape lets SQLite
// drive from `entries` (its index already yields `seq` order, so a following
// `ORDER BY seq` costs no sort) and then full-scan the unindexed `path`
// ephemeral table once per entry — O(n²). Measured on 8 000 entries: 53 s that
// way, 33 ms with `IN`, for the same rows.
const PATH_CTE: &str = "WITH RECURSIVE path(seq) AS (
                 SELECT COALESCE(?2, (SELECT leaf FROM sessions WHERE id = ?1))
                 UNION ALL
                 SELECT e.parent_seq FROM entries e
                 JOIN path p ON e.session_id = ?1 AND e.seq = p.seq
             )";

pub struct Store {
    conn: Connection,
}

impl Store {
    // Open (or create) the DB and run migrations. File lives in the user data directory.
    pub fn open(path: &std::path::Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create data directory {}", dir.display()))?;
        }
        let conn = Connection::open(path)
            .with_context(|| format!("cannot open database {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id          INTEGER PRIMARY KEY,
                name        TEXT,
                started_at  TEXT NOT NULL,
                cwd         TEXT
            );
            CREATE TABLE IF NOT EXISTS entries (
                session_id  INTEGER NOT NULL REFERENCES sessions(id),
                seq         INTEGER NOT NULL,
                ts          TEXT NOT NULL,
                kind        TEXT NOT NULL,
                payload     TEXT NOT NULL,
                PRIMARY KEY (session_id, seq)
            );
            CREATE TABLE IF NOT EXISTS cwd_history (
                session_id  INTEGER NOT NULL REFERENCES sessions(id),
                seq         INTEGER NOT NULL,
                ts          TEXT NOT NULL,
                cwd         TEXT NOT NULL,
                PRIMARY KEY (session_id, seq)
            );
            CREATE TABLE IF NOT EXISTS artifacts (
                id          INTEGER PRIMARY KEY,
                session_id  INTEGER NOT NULL REFERENCES sessions(id),
                name        TEXT NOT NULL,
                total_lines INTEGER NOT NULL,
                content     TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS rounds (
                session_id  INTEGER NOT NULL REFERENCES sessions(id),
                seq         INTEGER NOT NULL,
                ts          TEXT NOT NULL,
                model       TEXT NOT NULL,
                protocol    TEXT NOT NULL,
                base_url    TEXT NOT NULL,
                system      TEXT NOT NULL,
                tools_json  TEXT NOT NULL,
                max_tokens  INTEGER NOT NULL,
                stop_reason TEXT,
                first_seq   INTEGER,
                last_seq    INTEGER,
                PRIMARY KEY (session_id, seq)
            );",
        )?;
        // Schema upgrade: add the cwd column to old databases (NULL = unrecorded)
        let has_cwd: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = 'cwd'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .unwrap_or(false);
        if !has_cwd {
            conn.execute_batch("ALTER TABLE sessions ADD COLUMN cwd TEXT;")?;
        }
        // Tree layout (append-only): entries point at their parent row;
        // sessions.leaf names the tip that the next append hangs from.
        // NULL parent_seq on row seq=1 means "root"; on any later row it is
        // a *deliberate* second root (set_leaf(None) + append), not a legacy
        // record — the two are told apart by whether the column existed
        // before this open.
        let has_parent: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('entries') WHERE name = 'parent_seq'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .unwrap_or(false);
        let has_leaf: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('sessions') WHERE name = 'leaf'",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .unwrap_or(false);
        // A database missing either column predates the tree layout: its
        // rows are one linear chain and must be backfilled.
        let legacy_linear = !has_parent || !has_leaf;
        if !has_parent {
            conn.execute_batch("ALTER TABLE entries ADD COLUMN parent_seq INTEGER;")?;
        }
        if !has_leaf {
            conn.execute_batch("ALTER TABLE sessions ADD COLUMN leaf INTEGER;")?;
        }
        // Backfill **once**, on the upgrade that added the columns. Running it
        // on every open (as it did) destroyed user state two ways: a rewound
        // `leaf` snapped back to MAX(seq) — the tip of the branch the user had
        // just left — and every second root (parent_seq NULL, seq > 1) was
        // welded onto the previous row, merging two branches into one chain.
        if legacy_linear {
            conn.execute_batch(
                "UPDATE entries SET parent_seq = seq - 1
                 WHERE parent_seq IS NULL AND seq > 1;
                 UPDATE sessions SET leaf = (
                     SELECT MAX(seq) FROM entries e WHERE e.session_id = sessions.id);",
            )?;
        }
        Ok(Self { conn })
    }

    // Create a session, returning its id. `started_at` is written when
    // the first turn starts; `cwd` records the initial working directory.
    pub fn create_session(&mut self, started_at: &str, cwd: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO sessions (started_at, cwd) VALUES (?1, ?2)",
            [started_at, cwd],
        )?;
        let id = self.conn.last_insert_rowid();
        // The initial directory is itself a cwd history entry (seq 0 = origin)
        self.record_cwd(id, 0, cwd)?;
        Ok(id)
    }

    // /cdp: permanent migration — update sessions.cwd and append a
    // history row. Every migration is recorded so resume can land on
    // any of them.
    pub fn record_cwd(&mut self, session_id: i64, seq: i64, cwd: &str) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO cwd_history (session_id, seq, ts, cwd) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id, seq) DO UPDATE SET cwd = excluded.cwd, ts = excluded.ts",
            rusqlite::params![session_id, seq, now_stamp(), cwd],
        )?;
        tx.execute(
            "UPDATE sessions SET cwd = ?1 WHERE id = ?2",
            rusqlite::params![cwd, session_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    // /name: set or clear (None) the session name.
    pub fn set_session_name(&mut self, session_id: i64, name: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE sessions SET name = ?1 WHERE id = ?2",
            rusqlite::params![name, session_id],
        )?;
        Ok(())
    }

    // Append a whole round of entries in one transaction, with seq
    // allocated contiguously from the current max + 1. All-or-nothing:
    // the round is either fully stored or not at all.
    pub fn append(&mut self, session_id: i64, entries: &[crate::server::entry::Entry]) -> Result<()> {
        let tx = self.conn.transaction()?;
        Self::append_in(&tx, session_id, entries)?;
        tx.commit()?;
        Ok(())
    }

    // The entry-append body, on a caller-owned transaction: everything that
    // writes entries shares this so "hangs off the leaf / advances the leaf"
    // can never diverge between the round path and the marker path.
    //
    // Returns the (first, last) allocated seq — the span a round row records.
    fn append_in(
        tx: &rusqlite::Transaction<'_>,
        session_id: i64,
        entries: &[crate::server::entry::Entry],
    ) -> Result<(Option<i64>, Option<i64>)> {
        // The whole round hangs off the current leaf, and the leaf
        // advances to the last appended row — one transaction makes
        // "where is the tip" and "what was appended" atomic.
        let leaf: Option<i64> = tx
            .query_row(
                "SELECT leaf FROM sessions WHERE id = ?1",
                [session_id],
                |r| r.get(0),
            )
            .context("failed to query leaf")?;
        let next: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM entries WHERE session_id = ?1",
                [session_id],
                |r| r.get(0),
            )
            .context("failed to query max seq")?;
        if entries.is_empty() {
            return Ok((None, None));
        }
        let ts = now_stamp();
        let mut parent = leaf;
        for (i, e) in entries.iter().enumerate() {
            let (kind, payload) = e.to_payload();
            tx.execute(
                "INSERT INTO entries (session_id, seq, ts, kind, payload, parent_seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![session_id, next + i as i64, ts, kind, payload, parent],
            )?;
            parent = Some(next + i as i64);
        }
        tx.execute(
            "UPDATE sessions SET leaf = ?1 WHERE id = ?2",
            rusqlite::params![parent, session_id],
        )?;
        Ok((Some(next), Some(next + entries.len() as i64 - 1)))
    }

    // Persist one round **and its request header** in a single transaction.
    //
    // The header is what makes a stored conversation byte-reproducible from
    // the database alone: the system prompt, the tool manuals, the model id,
    // the endpoint and the token ceiling that the request actually carried.
    // None of it may be re-derived from local config/profile at read time —
    // the DB is the only source, so a different machine, a different UI or an
    // edited config still replays the exact bytes that went to the gateway.
    //
    // `stop` is the round's end reason (`StopReason::as_str`), or `None` for
    // a round that died before the gateway answered (mid-flight error): the
    // partial reply is still stored, and a NULL stop_reason is the honest
    // record of "we never got a final stop".
    pub fn append_round(
        &mut self,
        session_id: i64,
        entries: &[crate::server::entry::Entry],
        meta: &RoundMeta,
        stop: Option<&str>,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        let (first_seq, last_seq) = Self::append_in(&tx, session_id, entries)?;
        let seq: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM rounds WHERE session_id = ?1",
                [session_id],
                |r| r.get(0),
            )
            .context("failed to query max round seq")?;
        tx.execute(
            "INSERT INTO rounds (session_id, seq, ts, model, protocol, base_url, system,
                                 tools_json, max_tokens, stop_reason, first_seq, last_seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            rusqlite::params![
                session_id,
                seq,
                now_stamp(),
                meta.model,
                meta.protocol,
                meta.base_url,
                meta.system,
                meta.tools_json,
                meta.max_tokens,
                stop,
                first_seq,
                last_seq
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    // Every stored request header of a session, oldest first.
    pub fn rounds(&self, session_id: i64) -> Result<Vec<RoundRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, ts, model, protocol, base_url, system, tools_json, max_tokens,
                    stop_reason, first_seq, last_seq
             FROM rounds WHERE session_id = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map([session_id], |r| {
            Ok(RoundRow {
                seq: r.get(0)?,
                ts: r.get(1)?,
                model: r.get(2)?,
                protocol: r.get(3)?,
                base_url: r.get(4)?,
                system: r.get(5)?,
                tools_json: r.get(6)?,
                max_tokens: r.get(7)?,
                stop_reason: r.get(8)?,
                first_seq: r.get(9)?,
                last_seq: r.get(10)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    // One stored request header, or None when the round is not stored.
    pub fn round(&self, session_id: i64, seq: i64) -> Result<Option<RoundRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, ts, model, protocol, base_url, system, tools_json, max_tokens,
                    stop_reason, first_seq, last_seq
             FROM rounds WHERE session_id = ?1 AND seq = ?2",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![session_id, seq], |r| {
            Ok(RoundRow {
                seq: r.get(0)?,
                ts: r.get(1)?,
                model: r.get(2)?,
                protocol: r.get(3)?,
                base_url: r.get(4)?,
                system: r.get(5)?,
                tools_json: r.get(6)?,
                max_tokens: r.get(7)?,
                stop_reason: r.get(8)?,
                first_seq: r.get(9)?,
                last_seq: r.get(10)?,
            })
        })?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    // Load the projected path (root → leaf) of a session. The tree is
    // append-only, so "the conversation" is whatever chain the leaf
    // pointer currently names; abandoned branches stay stored but off-path.
    pub fn load_entries(&self, session_id: i64) -> Result<Vec<crate::server::entry::Entry>> {
        Ok(self.load_path(session_id, None)?.0)
    }

    // Load a path as (readable entries, seqs of rows whose payload could not
    // be parsed).
    //
    // The unreadable list exists because the two callers want opposite things:
    // a UI must degrade gracefully (skip a row it does not understand, forward
    // compatibility), while **replay must not** — silently dropping one row
    // changes the request bytes, which is exactly what the stored round is
    // supposed to make impossible. So the information is returned instead of
    // discarded here.
    pub fn load_path(
        &self,
        session_id: i64,
        leaf: Option<i64>,
    ) -> Result<(Vec<crate::server::entry::Entry>, Vec<i64>)> {
        let sql = format!(
            "{PATH_CTE}
             SELECT e.seq, e.kind, e.payload FROM entries e
             WHERE e.session_id = ?1 AND e.seq IN (SELECT seq FROM path)
             ORDER BY e.seq"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params![session_id, leaf], |r| {
            let seq: i64 = r.get(0)?;
            let kind: String = r.get(1)?;
            let payload: String = r.get(2)?;
            Ok((seq, kind, payload))
        })?;
        let mut out = Vec::new();
        let mut missing = Vec::new();
        for row in rows {
            let (seq, kind, payload) = row?;
            match crate::server::entry::Entry::from_payload(&kind, &payload) {
                Some(e) => out.push(e),
                // Unknown kind: skip rather than fail the whole session —
                // forward compatibility
                None => missing.push(seq),
            }
        }
        Ok((out, missing))
    }

    // Move the tip to an arbitrary stored row. Nothing is deleted: the old
    // branch stays reachable by re-pointing the leaf at any of its rows.
    // `None` rewinds to the root (before the first entry).
    pub fn set_leaf(&mut self, session_id: i64, seq: Option<i64>) -> Result<()> {
        if let Some(s) = seq {
            let exists: bool = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM entries WHERE session_id = ?1 AND seq = ?2",
                    rusqlite::params![session_id, s],
                    |r| r.get::<_, i64>(0),
                )
                .map(|n| n > 0)?;
            if !exists {
                anyhow::bail!("no entry seq {} in session {}", s, session_id);
            }
        }
        self.conn.execute(
            "UPDATE sessions SET leaf = ?1 WHERE id = ?2",
            rusqlite::params![seq, session_id],
        )?;
        Ok(())
    }

    // Flat view of every stored row of a session, for the tree picker:
    // (seq, parent_seq, kind, payload) in seq order. Branch structure is
    // visible directly through parent_seq.
    pub fn load_tree(&self, session_id: i64) -> Result<Vec<TreeNode>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, parent_seq, kind, payload FROM entries
             WHERE session_id = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map([session_id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (seq, parent_seq, kind, payload) = row?;
            out.push(TreeNode {
                seq,
                parent_seq,
                kind,
                payload,
            });
        }
        Ok(out)
    }

    // Effective session name: the nearest `name` marker looking back from the
    // leaf (pi's session_info semantic — a branch inherits the name it was
    // created under, siblings never see it). Falls back to the legacy
    // sessions.name column.
    //
    // One row, not the whole transcript: this runs on every resume, right
    // after `load_path`, and materializing every entry just to find a marker
    // would put the O(n²) read back on the hot path by itself.
    pub fn effective_name(&self, session_id: i64) -> Result<Option<String>> {
        let sql = format!(
            "{PATH_CTE}
             SELECT e.payload FROM entries e
             WHERE e.session_id = ?1 AND e.kind = 'name' AND e.seq IN (SELECT seq FROM path)
             ORDER BY e.seq DESC LIMIT 1"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let mut rows = stmt.query_map(rusqlite::params![session_id, Option::<i64>::None], |r| {
            r.get::<_, String>(0)
        })?;
        if let Some(payload) = rows.next() {
            let payload = payload?;
            if let Some(crate::server::entry::Entry::Name { name }) =
                crate::server::entry::Entry::from_payload("name", &payload)
            {
                return Ok(Some(name));
            }
        }
        self.conn
            .query_row(
                "SELECT name FROM sessions WHERE id = ?1",
                [session_id],
                |r| r.get(0),
            )
            .map_err(Into::into)
    }

    // Current tip row of a session.
    pub fn get_leaf(&self, session_id: i64) -> Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT leaf FROM sessions WHERE id = ?1",
                [session_id],
                |r| r.get(0),
            )
            .map_err(Into::into)
    }

    // List all sessions, newest first. Used by the resume picker.
    pub fn list_sessions(&self) -> Result<Vec<SessionMeta>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, started_at, cwd FROM sessions ORDER BY id DESC")?;
        let rows = stmt.query_map([], |r| {
            Ok(SessionMeta {
                id: r.get(0)?,
                name: r.get(1)?,
                started_at: r.get(2)?,
                cwd: r.get(3)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// Rows for the resume picker, newest first.
    ///
    /// `under = Some(dir)`: only sessions that ever ran in that directory —
    /// project A never sees project B's sessions (`/resume` opens in that
    /// scope; Tab widens it). `None` = every session on this machine.
    ///
    /// Everything the picker draws comes back in **one** query: the first
    /// user message (preview line + fallback display name) and the stored
    /// byte count (how big the conversation is). A per-row round trip would
    /// make a hundred-session list a hundred queries.
    pub fn list_session_rows(&self, under: Option<&std::path::Path>) -> Result<Vec<SessionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.name, s.started_at, s.cwd,
                    (SELECT e.payload FROM entries e
                      WHERE e.session_id = s.id AND e.kind = 'user'
                      ORDER BY e.seq LIMIT 1),
                    (SELECT COALESCE(SUM(LENGTH(e.payload)), 0) FROM entries e
                      WHERE e.session_id = s.id)
             FROM sessions s
             WHERE ?1 IS NULL
                OR s.id IN (SELECT DISTINCT session_id FROM cwd_history WHERE cwd = ?1)
             ORDER BY s.id DESC",
        )?;
        let under = under.map(|p| p.to_string_lossy().to_string());
        let rows = stmt.query_map(rusqlite::params![under], |r| {
            let payload: Option<String> = r.get(4)?;
            Ok(SessionRow {
                meta: SessionMeta {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    started_at: r.get(2)?,
                    cwd: r.get(3)?,
                },
                first_user: payload.as_deref().and_then(first_user_text),
                bytes: r.get(5)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    // Sessions created under a project directory (matched against
    // cwd_history), newest first. /resume only shows sessions of the
    // current directory — project A never sees project B's sessions.
    pub fn list_sessions_under(&self, root: &std::path::Path) -> Result<Vec<SessionMeta>> {
        let root_str = root.to_string_lossy();
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.name, s.started_at, s.cwd FROM sessions s
             WHERE s.id IN (
                 SELECT DISTINCT session_id FROM cwd_history WHERE cwd = ?1
             ) ORDER BY s.id DESC",
        )?;
        let rows = stmt.query_map([&root_str], |r| {
            Ok(SessionMeta {
                id: r.get(0)?,
                name: r.get(1)?,
                started_at: r.get(2)?,
                cwd: r.get(3)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    // Fetch one session's metadata.
    pub fn session(&self, session_id: i64) -> Result<SessionMeta> {
        self.conn
            .query_row(
                "SELECT id, name, started_at, cwd FROM sessions WHERE id = ?1",
                [session_id],
                |r| {
                    Ok(SessionMeta {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        started_at: r.get(2)?,
                        cwd: r.get(3)?,
                    })
                },
            )
            .map_err(Into::into)
    }

    // The session cwd migration history (seq ascending). seq 0 = session origin.
    pub fn cwd_history(&self, session_id: i64) -> Result<Vec<(i64, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT seq, cwd FROM cwd_history WHERE session_id = ?1 ORDER BY seq")?;
        let rows = stmt.query_map([session_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    // ---- artifacts: oversized tool outputs stored out-of-band ----
    //
    // A tool result above the threshold is stored here whole; the context
    // gets a one-line placeholder with the artifact id, and the model pulls
    // content back through `#id` virtual files in bash. Sessions delete
    // their artifacts with them.

    /// Store an oversized tool output; returns its artifact id.
    pub fn put_artifact(&mut self, session_id: i64, name: &str, content: &str) -> Result<i64> {
        let lines = content.lines().count() as i64;
        self.conn.execute(
            "INSERT INTO artifacts (session_id, name, total_lines, content) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![session_id, name, lines, content],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Fetch one artifact (whole content + metadata). Caller checks the
    /// session id owns it before handing bytes to anyone.
    pub fn get_artifact(&self, id: i64, session_id: i64) -> Result<Option<(String, i64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, total_lines, content FROM artifacts WHERE id = ?1 AND session_id = ?2",
        )?;
        let mut rows = stmt.query_map([id, session_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    /// Delete a session and everything hanging off it (entries, cwd
    /// history, artifacts). Returns whether a session row was removed.
    pub fn delete_session(&mut self, session_id: i64) -> Result<bool> {
        let tx = self.conn.transaction()?;
        for table in ["entries", "cwd_history", "artifacts", "rounds"] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE session_id = ?1"),
                [session_id],
            )?;
        }
        let n = tx.execute("DELETE FROM sessions WHERE id = ?1", [session_id])?;
        tx.commit()?;
        Ok(n > 0)
    }
}

// Local timestamp `YYYY-MM-DD HH:MM:SS`.
//
// No chrono dependency: the `date` command suffices, timestamps only
// affect display names and debugging, never logic comparisons.
pub fn now_stamp() -> String {
    std::process::Command::new("date")
        .args(["+%Y-%m-%d %H:%M:%S"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

/// Pull the text out of a stored `user` payload.
///
/// A row we cannot parse is not an error here: the picker shows the session
/// without a preview line rather than hiding it (forward compatibility, same
/// rule as `load_path`).
fn first_user_text(payload: &str) -> Option<String> {
    match crate::server::entry::Entry::from_payload("user", payload) {
        Some(crate::server::entry::Entry::User { content }) => Some(content),
        _ => None,
    }
}

/// Seconds since the local timestamp `stamp` (`YYYY-MM-DD HH:MM:SS`).
///
/// `None` when the stamp is malformed or `date` refused it — the picker then
/// shows the raw stamp instead of a wrong age. Same reasoning as `now_stamp`:
/// one `date` call beats a date-arithmetic dependency.
pub fn age_seconds(stamp: &str) -> Option<u64> {
    if stamp.is_empty() {
        return None;
    }
    let out = std::process::Command::new("date")
        .args(["-d", stamp, "+%s"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let then: u64 = String::from_utf8(out.stdout).ok()?.trim().parse().ok()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(now.saturating_sub(then))
}

// Display name in the resume picker: `MM-DD:HH-MMSS+first 7 chars` when unnamed.
pub fn display_name(meta: &SessionMeta, first_user: Option<&str>) -> String {
    if let Some(n) = &meta.name {
        return n.clone();
    }
    let prefix = first_user
        .map(|s| s.chars().take(7).collect::<String>())
        .unwrap_or_default();
    // started_at "2026-09-22 14:30:05" → "09-22:14-3005"
    // Sliced by **characters**: the timestamp is our own ASCII, but a
    // hand-edited or truncated row must not panic the picker.
    let t: Vec<char> = meta.started_at.chars().collect();
    let part = |a: usize, b: usize| t[a..b].iter().collect::<String>();
    if t.len() >= 19 {
        format!(
            "{}-{}:{}-{}{}+{}",
            part(5, 7),
            part(8, 10),
            part(11, 13),
            part(14, 16),
            part(17, 19),
            prefix
        )
    } else {
        format!("?+{prefix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::entry::Entry;

    fn mem_store() -> Store {
        // rusqlite in-memory database: path ":memory:"
        Store::open(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn append_and_load_round_trip() {
        let mut s = mem_store();
        let id = s.create_session("2026-09-22 14:30:05", "/tmp").unwrap();
        let entries = vec![
            Entry::User {
                content: "你好\n世界".into(),
            },
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "edit".into(),
                args: r#"{"path":"./a.txt"}"#.into(),
                intent: String::new(),
                text: String::new(),
                first: true,
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "edit".into(),
                ok: true,
                result: "已替换：./a.txt".into(),
                details: None,
                duration_ms: 0,
            },
            Entry::Assistant {
                content: "改好了".into(),
                usage: None,
            },
        ];
        s.append(id, &entries).unwrap();
        let back = s.load_entries(id).unwrap();
        assert_eq!(back, entries, "round-trip must be byte-identical");
    }

    #[test]
    fn seq_is_monotonic_across_turns() {
        let mut s = mem_store();
        let id = s.create_session("t", "/tmp").unwrap();
        s.append(
            id,
            &[Entry::User {
                content: "一".into(),
            }],
        )
        .unwrap();
        s.append(
            id,
            &[Entry::User {
                content: "二".into(),
            }],
        )
        .unwrap();
        let n: i64 = s
            .conn
            .query_row(
                "SELECT COUNT(*) FROM entries WHERE session_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 2);
        let max: i64 = s
            .conn
            .query_row(
                "SELECT MAX(seq) FROM entries WHERE session_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(max, 2, "seq must increment contiguously across rounds");
    }

    #[test]
    fn name_is_explicit_only() {
        let mut s = mem_store();
        let id = s.create_session("t", "/tmp").unwrap();
        let meta = s.session(id).unwrap();
        assert_eq!(meta.name, None, "unnamed by default");
        s.set_session_name(id, Some("鲤鱼会")).unwrap();
        let meta = s.session(id).unwrap();
        assert_eq!(meta.name.as_deref(), Some("鲤鱼会"));
    }

    #[test]
    fn display_name_follows_the_spec() {
        let m = SessionMeta {
            cwd: None,
            id: 1,
            name: None,
            started_at: "2026-09-22 14:30:05".into(),
        };
        assert_eq!(
            display_name(&m, Some("红鲤鱼与绿鲤鱼")),
            "09-22:14-3005+红鲤鱼与绿鲤鱼"
        );
        let named = SessionMeta {
            cwd: None,
            id: 1,
            name: Some("正式名".into()),
            started_at: "2026-09-22 14:30:05".into(),
        };
        assert_eq!(display_name(&named, None), "正式名");
    }

    #[test]
    fn unknown_kind_is_skipped_not_fatal() {
        let mut s = mem_store();
        let id = s.create_session("t", "/tmp").unwrap();
        s.append(
            id,
            &[Entry::User {
                content: "x".into(),
            }],
        )
        .unwrap();
        // Insert an unknown kind by hand (simulating a future version's write)
        s.conn
            .execute(
                "INSERT INTO entries (session_id, seq, ts, kind, payload) VALUES (?1, 99, 't', 'brand_new_kind', '{}')",
                [id],
            )
            .unwrap();
        let entries = s.load_entries(id).unwrap();
        assert_eq!(entries.len(), 1, "unknown kind skipped, rest unaffected");
    }

    #[test]
    fn the_picker_rows_carry_a_preview_and_a_size() {
        let mut s = mem_store();
        let a = s.create_session("2026-09-22 14:30:05", "/proj/a").unwrap();
        let b = s.create_session("2026-09-22 15:30:05", "/proj/b").unwrap();
        s.record_cwd(a, 1, "/proj/shared").unwrap();
        s.append(
            a,
            &[
                Entry::User {
                    content: "改一下 todo\n第二行不该出现在预览里".into(),
                },
                Entry::Assistant {
                    content: "好".into(),
                    usage: None,
                },
            ],
        )
        .unwrap();

        let all = s.list_session_rows(None).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].meta.id, b, "新的在前");
        let row_a = all.iter().find(|r| r.meta.id == a).unwrap();
        assert_eq!(
            row_a.first_user.as_deref(),
            Some("改一下 todo\n第二行不该出现在预览里"),
            "原文照给，换行由渲染层处理"
        );
        assert!(row_a.bytes > 0, "有内容就该有体积");
        let row_b = all.iter().find(|r| r.meta.id == b).unwrap();
        assert_eq!(row_b.first_user, None, "空会话没有可预览的");
        assert_eq!(row_b.bytes, 0);

        // 项目作用域：会话 a 在 /proj/shared 跑过（cwd_history 记着），
        // b 没有——项目 A 不该看见项目 B 的会话。
        let shared = s.list_session_rows(Some(std::path::Path::new("/proj/shared"))).unwrap();
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].meta.id, a);
        assert!(
            s.list_session_rows(Some(std::path::Path::new("/elsewhere")))
                .unwrap()
                .is_empty(),
            "没跑过的目录是空列表，不是错误"
        );
    }

    #[test]
    fn an_age_is_reported_and_garbage_falls_back_to_nothing() {
        // 合法戳：秒数是"过去"，不会倒挂。
        let recent = now_stamp();
        let age = age_seconds(&recent).expect("刚刚写的戳算得出来");
        assert!(age < 60, "刚生成的戳不该有几秒以上的年龄：{age}");
        assert_eq!(age_seconds(""), None);
        assert_eq!(age_seconds("不是时间"), None);
    }

    #[test]
    fn cwd_history_records_permanent_moves() {
        let mut s = mem_store();
        let id = s.create_session("t", "/home/u/proj").unwrap();
        s.record_cwd(id, 1, "/tmp/clone-a").unwrap();
        s.record_cwd(id, 2, "/tmp/clone-b").unwrap();
        let h = s.cwd_history(id).unwrap();
        assert_eq!(h.len(), 3, "origin + two migrations");
        assert_eq!(h[0], (0, "/home/u/proj".to_string()));
        assert_eq!(h[2], (2, "/tmp/clone-b".to_string()));
        // sessions.cwd must point at the final landing spot
        let meta = s.session(id).unwrap();
        assert_eq!(meta.cwd.as_deref(), Some("/tmp/clone-b"));
    }

    #[test]
    fn list_sessions_under_filters_by_project() {
        let mut s = mem_store();
        let a = s.create_session("t", "/home/u/projA").unwrap();
        let _b = s.create_session("t", "/home/u/projB").unwrap();
        let under_a = s
            .list_sessions_under(std::path::Path::new("/home/u/projA"))
            .unwrap();
        assert_eq!(under_a.len(), 1, "only sessions created under projA");
        assert_eq!(under_a[0].id, a);
    }
}

#[cfg(test)]
mod tree_tests {
    use super::*;
    use crate::server::entry::Entry;

    fn mem() -> Store {
        let dir = std::env::temp_dir().join(format!(
            "mypi-tree-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Store::open(&dir.join("t.db")).unwrap()
    }

    fn user(t: &str) -> Entry {
        Entry::User { content: t.into() }
    }
    fn assistant(t: &str) -> Entry {
        Entry::Assistant {
            content: t.into(),
            usage: None,
        }
    }

    #[test]
    fn append_hangs_off_leaf_and_advances_it() {
        let mut s = mem();
        let id = s.create_session("t", "/").unwrap();
        s.append(id, &[user("a")]).unwrap();
        s.append(id, &[assistant("b"), user("c")]).unwrap();
        // Root -> a -> b -> c
        let leaf = s.get_leaf(id).unwrap();
        let tree = s.load_tree(id).unwrap();
        assert_eq!(leaf, Some(3));
        assert_eq!(tree.len(), 3);
        assert_eq!(tree[0].parent_seq, None); // root has no parent
        assert_eq!(tree[1].parent_seq, Some(1)); // b hangs off a
        assert_eq!(tree[2].parent_seq, Some(2)); // c hangs off b
    }

    #[test]
    fn branch_by_set_leaf_keeps_old_branch_rows() {
        let mut s = mem();
        let id = s.create_session("t", "/").unwrap();
        s.append(id, &[user("a")]).unwrap();
        s.append(id, &[assistant("old-1"), user("old-2")]).unwrap();
        // Rewind to seq 1 (after "a") and start a new branch.
        s.set_leaf(id, Some(1)).unwrap();
        s.append(id, &[assistant("new-1")]).unwrap();
        // Projected path: a, new-1. Old branch rows still exist on disk.
        let entries = s.load_entries(id).unwrap();
        let texts: Vec<&str> = entries
            .iter()
            .map(|e| match e {
                Entry::User { content } | Entry::Assistant { content, .. } => content.as_str(),
                _ => "",
            })
            .collect();
        assert_eq!(texts, vec!["a", "new-1"]);
        let tree = s.load_tree(id).unwrap();
        assert_eq!(tree.len(), 4); // nothing deleted
        assert_eq!(tree[3].parent_seq, Some(1)); // new-1 forks from a
        // Old branch still fully loadable by pointing back at it.
        s.set_leaf(id, Some(3)).unwrap();
        let texts_old: Vec<String> = s
            .load_entries(id)
            .unwrap()
            .iter()
            .map(|e| match e {
                Entry::User { content } | Entry::Assistant { content, .. } => content.clone(),
                _ => String::new(),
            })
            .collect();
        assert_eq!(texts_old, vec!["a", "old-1", "old-2"]);
    }

    #[test]
    fn set_leaf_root_then_append_starts_new_root() {
        let mut s = mem();
        let id = s.create_session("t", "/").unwrap();
        s.append(id, &[user("a")]).unwrap();
        s.set_leaf(id, None).unwrap();
        s.append(id, &[user("b")]).unwrap();
        let tree = s.load_tree(id).unwrap();
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[1].parent_seq, None); // second root — pi's resetLeaf semantic
        let entries = s.load_entries(id).unwrap();
        let texts: Vec<&str> = entries
            .iter()
            .map(|e| match e {
                Entry::User { content } => content.as_str(),
                _ => "",
            })
            .collect();
        assert_eq!(texts, vec!["b"]);
    }

    #[test]
    fn legacy_linear_db_is_backfilled_on_open() {
        let dir = std::env::temp_dir().join(format!("mypi-legacy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("l.db");
        // Simulate a pre-tree database: no parent_seq, no leaf columns.
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE sessions (id INTEGER PRIMARY KEY, name TEXT, started_at TEXT NOT NULL, cwd TEXT);
             CREATE TABLE entries (session_id INTEGER NOT NULL, seq INTEGER NOT NULL, ts TEXT NOT NULL, kind TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY (session_id, seq));
             INSERT INTO sessions (id, started_at) VALUES (1, 't');
             INSERT INTO entries VALUES (1, 1, 't', 'user', '{\"content\":\"a\"}');
             INSERT INTO entries VALUES (1, 2, 't', 'assistant', '{\"content\":\"b\"}');",
        ).unwrap();
        drop(conn);
        let s = Store::open(&db).unwrap();
        // Projection over backfilled parents yields the original linear order.
        let entries = s.load_entries(1).unwrap();
        let texts: Vec<&str> = entries
            .iter()
            .map(|e| match e {
                Entry::User { content } => content.as_str(),
                Entry::Assistant { content, .. } => content.as_str(),
                _ => "",
            })
            .collect();
        assert_eq!(texts, vec!["a", "b"]);
        assert_eq!(s.get_leaf(1).unwrap(), Some(2));
    }

    /// File-backed store + its path, so a test can close and reopen it.
    fn file_store(tag: &str) -> (Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "mypi-reopen-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("t.db");
        (Store::open(&db).unwrap(), db)
    }

    fn texts_of(s: &Store, id: i64) -> Vec<String> {
        s.load_entries(id)
            .unwrap()
            .iter()
            .map(|e| match e {
                Entry::User { content } | Entry::Assistant { content, .. } => content.clone(),
                _ => String::new(),
            })
            .collect()
    }

    #[test]
    fn a_rewound_leaf_survives_a_reopen() {
        // The bug this guards: `Store::open` re-ran the legacy backfill on
        // every open, snapping `leaf` back to MAX(seq) — the tip of the very
        // branch the user had rewound away from. The next append then hung off
        // the wrong branch, resurrecting messages the user had left behind.
        let (mut s, db) = file_store("leaf");
        let id = s.create_session("t", "/").unwrap();
        s.append(id, &[user("a")]).unwrap();
        s.append(id, &[assistant("old-1"), user("old-2")]).unwrap();
        s.set_leaf(id, Some(1)).unwrap();
        assert_eq!(s.get_leaf(id).unwrap(), Some(1));
        drop(s);

        let mut s = Store::open(&db).unwrap();
        assert_eq!(
            s.get_leaf(id).unwrap(),
            Some(1),
            "rewound leaf must survive a restart"
        );
        // The next append forks off the rewound tip, not off the tail.
        s.append(id, &[assistant("new-1")]).unwrap();
        assert_eq!(texts_of(&s, id), vec!["a", "new-1"]);
        // Nothing was deleted: the abandoned branch is still stored.
        assert_eq!(s.load_tree(id).unwrap().len(), 4);
    }

    #[test]
    fn a_null_leaf_means_root_and_survives_a_reopen() {
        let (mut s, db) = file_store("nullleaf");
        let id = s.create_session("t", "/").unwrap();
        s.append(id, &[user("a")]).unwrap();
        s.set_leaf(id, None).unwrap();
        drop(s);

        let s = Store::open(&db).unwrap();
        assert_eq!(
            s.get_leaf(id).unwrap(),
            None,
            "a restart must not resurrect the newest row over a rewound-to-root leaf"
        );
        assert!(s.load_entries(id).unwrap().is_empty());
    }

    #[test]
    fn a_deliberate_second_root_is_not_welded_on_reopen() {
        // Same root cause, second symptom: `UPDATE ... SET parent_seq = seq - 1
        // WHERE parent_seq IS NULL AND seq > 1` turned every deliberate second
        // root into a child of the previous row, merging two branches into one
        // linear chain (and making the abandoned branch replay into the model).
        let (mut s, db) = file_store("root");
        let id = s.create_session("t", "/").unwrap();
        s.append(id, &[user("a")]).unwrap();
        s.set_leaf(id, None).unwrap();
        s.append(id, &[user("b")]).unwrap();
        assert_eq!(s.load_tree(id).unwrap()[1].parent_seq, None);
        drop(s);

        let s = Store::open(&db).unwrap();
        let tree = s.load_tree(id).unwrap();
        assert_eq!(
            tree[1].parent_seq, None,
            "a second root must stay a root across a restart"
        );
        assert_eq!(
            texts_of(&s, id),
            vec!["b"],
            "the projection is the new root only"
        );
    }

    #[test]
    fn a_stored_round_replays_byte_identically_after_a_reopen() {
        // The database's whole contract: reopen the file and the conversation
        // comes back as the exact bytes the model saw — no drift from the
        // migration path.
        let (mut s, db) = file_store("bytes");
        let id = s.create_session("t", "/").unwrap();
        let round = vec![
            Entry::User {
                content: "问一下".into(),
            },
            Entry::Reasoning {
                content: "先想\n再想".into(),
            },
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "bash".into(),
                args: r#"{"command":"ls"}"#.into(),
                intent: "列目录".into(),
                text: "我看看".into(),
                first: true,
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "bash".into(),
                ok: true,
                result: "a.txt".into(),
                details: None,
                duration_ms: 0,
            },
            Entry::Assistant {
                content: "好了".into(),
                usage: None,
            },
        ];
        s.append(id, &round).unwrap();
        drop(s);

        let s = Store::open(&db).unwrap();
        let back = s.load_entries(id).unwrap();
        assert_eq!(back, round, "reopen must be byte-identical");
        let ctx = crate::server::turn::entries_to_context("sys", &back);
        let wire = serde_json::to_string(&ctx.messages).unwrap();
        assert!(wire.contains(r#"{\"command\":\"ls\"}"#), "{wire}");
        assert!(
            !wire.contains("先想"),
            "reasoning stays out of the protocol"
        );
    }
}

#[cfg(test)]
mod name_tests {
    use super::*;
    use crate::server::entry::Entry;

    #[test]
    fn name_marker_round_trips_and_resolves() {
        let dir = std::env::temp_dir().join(format!(
            "mypi-name-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Store::open(&dir.join("n.db")).unwrap();
        let id = s.create_session("t", "/").unwrap();
        s.append(
            id,
            &[Entry::User {
                content: "a".into(),
            }],
        )
        .unwrap();
        s.append(
            id,
            &[Entry::Name {
                name: "我的分支".into(),
            }],
        )
        .unwrap();
        s.append(
            id,
            &[Entry::Assistant {
                content: "b".into(),
                usage: None,
            }],
        )
        .unwrap();
        // Projection skips name in protocol; effective_name picks the nearest marker.
        assert_eq!(s.effective_name(id).unwrap().as_deref(), Some("我的分支"));
        // Round-trip through payload
        let (kind, payload) = Entry::Name { name: "x".into() }.to_payload();
        assert_eq!(
            Entry::from_payload(kind, &payload).unwrap(),
            Entry::Name { name: "x".into() }
        );
    }
}

// ---- 数据库位置（XDG）与旧布局迁移 ----

// Database location (XDG Data spec): $XDG_DATA_HOME/mypi/sessions.db,
// i.e. ~/.local/share/mypi/sessions.db by default. Legacy layout support:
// `~/.local/share/mypi` used to be the SQLite file itself — renamed to
// sessions.db on first open of the new layout.
pub fn db_path() -> std::path::PathBuf {
    crate::xdg::data_dir().join("sessions.db")
}

/// Migrate the legacy database file (a bare `mypi` file under
/// ~/.local/share) to the canonical `mypi/sessions.db` layout. No-op when
/// the legacy file is absent or already migrated.
pub fn migrate_legacy_db(base: &std::path::Path) {
    let legacy = base.join("mypi");
    if !legacy.is_file() {
        return;
    }
    // The legacy file **occupies the directory's name**, so: move the db
    // and its WAL/SHM companions out of the way, create the real
    // directory, then move them in as `sessions.db*`.
    let stash = base.join(".mypi-legacy-migrate");
    let _ = std::fs::remove_dir_all(&stash);
    std::fs::create_dir_all(&stash).ok();
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::rename(
            base.join(format!("mypi{suffix}")),
            stash.join(format!("db{suffix}")),
        );
    }
    if let Err(e) = std::fs::create_dir_all(base.join("mypi")) {
        eprintln!("mypi: cannot create data dir: {e}");
        return;
    }
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::rename(
            stash.join(format!("db{suffix}")),
            base.join(format!("mypi/sessions.db{suffix}")),
        );
    }
    let _ = std::fs::remove_dir_all(&stash);
}

// Session name for the statusline: an explicit /name wins; otherwise
// one is synthesized — first 7 chars of the first user message within the session.
