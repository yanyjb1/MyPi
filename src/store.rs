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
        let next: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM entries WHERE session_id = ?1",
                [session_id],
                |r| r.get(0),
            )
            .context("failed to query max seq")?;
        let ts = now_stamp();
        for (i, e) in entries.iter().enumerate() {
            let (kind, payload) = e.to_payload();
            tx.execute(
                "INSERT INTO entries (session_id, seq, ts, kind, payload) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![session_id, next + i as i64, ts, kind, payload],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // Load all entries of a session in seq order. Used by resume.
    pub fn load_entries(
        &self,
        session_id: i64,
    ) -> Result<Vec<crate::tui::components::chat::Entry>> {
        let mut stmt = self
            .conn
            .prepare("SELECT kind, payload FROM entries WHERE session_id = ?1 ORDER BY seq")?;
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
