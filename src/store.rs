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
        // NULL parent_seq on row seq=1 means "root"; on any later row it
        // marks a legacy linear record and is backfilled below.
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
        if !has_parent {
            conn.execute_batch("ALTER TABLE entries ADD COLUMN parent_seq INTEGER;")?;
        }
        if !has_leaf {
            conn.execute_batch("ALTER TABLE sessions ADD COLUMN leaf INTEGER;")?;
        }
        // Legacy linear rows: every non-root entry's parent is the previous seq.
        // Roots keep NULL. New rows always write parent_seq explicitly.
        conn.execute_batch(
            "UPDATE entries SET parent_seq = seq - 1
             WHERE parent_seq IS NULL AND seq > 1;
             UPDATE sessions SET leaf = COALESCE(
                 (SELECT MAX(seq) FROM entries e WHERE e.session_id = sessions.id), NULL);",
        )?;
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
    pub fn append(&mut self, session_id: i64, entries: &[crate::tui::components::chat::Entry]) -> Result<()> {
        let tx = self.conn.transaction()?;
        // The whole round hangs off the current leaf, and the leaf
        // advances to the last appended row — one transaction makes
        // "where is the tip" and "what was appended" atomic.
        let leaf: Option<i64> = tx
            .query_row("SELECT leaf FROM sessions WHERE id = ?1", [session_id], |r| r.get(0))
            .context("failed to query leaf")?;
        let next: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM entries WHERE session_id = ?1",
                [session_id],
                |r| r.get(0),
            )
            .context("failed to query max seq")?;
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
        tx.execute("UPDATE sessions SET leaf = ?1 WHERE id = ?2", rusqlite::params![parent, session_id])?;
        tx.commit()?;
        Ok(())
    }

    // Load the projected path (root → leaf) of a session. The tree is
    // append-only, so "the conversation" is whatever chain the leaf
    // pointer currently names; abandoned branches stay stored but off-path.
    pub fn load_entries(
        &self,
        session_id: i64,
    ) -> Result<Vec<crate::tui::components::chat::Entry>> {
        let mut stmt = self.conn.prepare(
            "WITH RECURSIVE path(seq) AS (
                 SELECT leaf FROM sessions WHERE id = ?1
                 UNION ALL
                 SELECT e.parent_seq FROM entries e
                 JOIN path p ON e.session_id = ?1 AND e.seq = p.seq
             )
             SELECT e.kind, e.payload FROM entries e
             JOIN path p ON e.seq = p.seq
             WHERE e.session_id = ?1 AND e.seq IS NOT NULL
             ORDER BY e.seq",
        )?;
        let rows = stmt.query_map([session_id], |r| {
            let kind: String = r.get(0)?;
            let payload: String = r.get(1)?;
            Ok((kind, payload))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (kind, payload) = row?;
            match crate::tui::components::chat::Entry::from_payload(&kind, &payload) {
                Some(e) => out.push(e),
                // Unknown kind: skip rather than fail the whole session —
                // forward compatibility
                None => continue,
            }
        }
        Ok(out)
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
            out.push(TreeNode { seq, parent_seq, kind, payload });
        }
        Ok(out)
    }

    // Effective session name: walk the projected path backwards and take
    // the nearest `name` entry (pi's session_info semantic). Falls back to
    // the legacy sessions.name column.
    pub fn effective_name(&self, session_id: i64) -> Result<Option<String>> {
        let entries = self.load_entries(session_id)?;
        for e in entries.iter().rev() {
            if let crate::tui::components::chat::Entry::Name { name } = e {
                return Ok(Some(name.clone()));
            }
        }
        self.conn
            .query_row("SELECT name FROM sessions WHERE id = ?1", [session_id], |r| r.get(0))
            .map_err(Into::into)
    }

    // Current tip row of a session.
    pub fn get_leaf(&self, session_id: i64) -> Result<Option<i64>> {
        self.conn
            .query_row("SELECT leaf FROM sessions WHERE id = ?1", [session_id], |r| r.get(0))
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
        let mut stmt = self.conn.prepare(
            "SELECT seq, cwd FROM cwd_history WHERE session_id = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map([session_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
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

// Display name in the resume picker: `MM-DD:HH-MMSS+first 7 chars` when unnamed.
pub fn display_name(meta: &SessionMeta, first_user: Option<&str>) -> String {
    if let Some(n) = &meta.name {
        return n.clone();
    }
    let prefix = first_user
        .map(|s| s.chars().take(7).collect::<String>())
        .unwrap_or_default();
    // started_at "2026-09-22 14:30:05" → "09-22:14-3005"
    let t = &meta.started_at;
    if t.len() >= 19 {
        format!(
            "{}-{}:{}-{}{}+{}",
            &t[5..7], &t[8..10], &t[11..13], &t[14..16], &t[17..19], prefix
        )
    } else {
        format!("?+{prefix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::components::chat::Entry;

    fn mem_store() -> Store {
        // rusqlite in-memory database: path ":memory:"
        Store::open(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn append_and_load_round_trip() {
        let mut s = mem_store();
        let id = s.create_session("2026-09-22 14:30:05", "/tmp").unwrap();
        let entries = vec![
            Entry::User { content: "你好\n世界".into() },
            Entry::ToolRequest { call_id: "c1".into(), name: "edit".into(), object: "./a.txt".into() },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "edit".into(),
                ok: true,
                result: "已替换：./a.txt".into(),
            },
            Entry::Assistant { content: "改好了".into(), usage: None, reasoning: None },
        ];
        s.append(id, &entries).unwrap();
        let back = s.load_entries(id).unwrap();
        assert_eq!(back, entries, "round-trip must be byte-identical");
    }

    #[test]
    fn seq_is_monotonic_across_turns() {
        let mut s = mem_store();
        let id = s.create_session("t", "/tmp").unwrap();
        s.append(id, &[Entry::User { content: "一".into() }]).unwrap();
        s.append(id, &[Entry::User { content: "二".into() }]).unwrap();
        let n: i64 = s.conn
            .query_row("SELECT COUNT(*) FROM entries WHERE session_id = ?1", [id], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2);
        let max: i64 = s.conn
            .query_row("SELECT MAX(seq) FROM entries WHERE session_id = ?1", [id], |r| r.get(0))
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
        let m = SessionMeta { cwd: None, id: 1, name: None, started_at: "2026-09-22 14:30:05".into() };
        assert_eq!(display_name(&m, Some("红鲤鱼与绿鲤鱼")),
                   "09-22:14-3005+红鲤鱼与绿鲤鱼");
        let named = SessionMeta { cwd: None, id: 1, name: Some("正式名".into()), started_at: "2026-09-22 14:30:05".into() };
        assert_eq!(display_name(&named, None), "正式名");
    }

    #[test]
    fn unknown_kind_is_skipped_not_fatal() {
        let mut s = mem_store();
        let id = s.create_session("t", "/tmp").unwrap();
        s.append(id, &[Entry::User { content: "x".into() }]).unwrap();
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
        let under_a = s.list_sessions_under(std::path::Path::new("/home/u/projA")).unwrap();
        assert_eq!(under_a.len(), 1, "only sessions created under projA");
        assert_eq!(under_a[0].id, a);
    }
}

#[cfg(test)]
mod tree_tests {
    use super::*;
    use crate::tui::components::chat::Entry;

    fn mem() -> Store {
        let dir = std::env::temp_dir().join(format!("mypi-tree-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        Store::open(&dir.join("t.db")).unwrap()
    }

    fn user(t: &str) -> Entry { Entry::User { content: t.into() } }
    fn assistant(t: &str) -> Entry { Entry::Assistant { content: t.into(), usage: None, reasoning: None } }

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
        assert_eq!(tree[0].parent_seq, None);     // root has no parent
        assert_eq!(tree[1].parent_seq, Some(1));  // b hangs off a
        assert_eq!(tree[2].parent_seq, Some(2));  // c hangs off b
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
        let texts: Vec<&str> = entries.iter().map(|e| match e {
            Entry::User { content } | Entry::Assistant { content, .. } => content.as_str(),
            _ => "",
        }).collect();
        assert_eq!(texts, vec!["a", "new-1"]);
        let tree = s.load_tree(id).unwrap();
        assert_eq!(tree.len(), 4);            // nothing deleted
        assert_eq!(tree[3].parent_seq, Some(1)); // new-1 forks from a
        // Old branch still fully loadable by pointing back at it.
        s.set_leaf(id, Some(3)).unwrap();
        let texts_old: Vec<String> = s.load_entries(id).unwrap().iter().map(|e| match e {
            Entry::User { content } | Entry::Assistant { content, .. } => content.clone(),
            _ => String::new(),
        }).collect();
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
        let texts: Vec<&str> = entries.iter().map(|e| match e {
            Entry::User { content } => content.as_str(),
            _ => "",
        }).collect();
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
        let texts: Vec<&str> = entries.iter().map(|e| match e {
            Entry::User { content } => content.as_str(),
            Entry::Assistant { content, .. } => content.as_str(),
            _ => "",
        }).collect();
        assert_eq!(texts, vec!["a", "b"]);
        assert_eq!(s.get_leaf(1).unwrap(), Some(2));
    }
}

#[cfg(test)]
mod name_tests {
    use super::*;
    use crate::tui::components::chat::Entry;

    #[test]
    fn name_marker_round_trips_and_resolves() {
        let dir = std::env::temp_dir().join(format!("mypi-name-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().subsec_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Store::open(&dir.join("n.db")).unwrap();
        let id = s.create_session("t", "/").unwrap();
        s.append(id, &[Entry::User { content: "a".into() }]).unwrap();
        s.append(id, &[Entry::Name { name: "我的分支".into() }]).unwrap();
        s.append(id, &[Entry::Assistant { content: "b".into(), usage: None, reasoning: None }]).unwrap();
        // Projection skips name in protocol; effective_name picks the nearest marker.
        assert_eq!(s.effective_name(id).unwrap().as_deref(), Some("我的分支"));
        // Round-trip through payload
        let (kind, payload) = Entry::Name { name: "x".into() }.to_payload();
        assert_eq!(Entry::from_payload(kind, &payload).unwrap(), Entry::Name { name: "x".into() });
    }
}
