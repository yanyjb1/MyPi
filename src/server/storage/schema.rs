//! Storage schema: the SQLite layout and the connection's three settings.
//! A version mismatch drops and rebuilds; there is no migration.

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Layout version. A mismatch wipes the database and rebuilds it.
pub const SCHEMA_VERSION: i64 = 14;

/// Open (or create) the database and make sure its layout is current.
pub fn prepare(path: &std::path::Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("cannot create data directory {}", dir.display()))?;
    }
    let conn = Connection::open(path)
        .with_context(|| format!("cannot open database {}", path.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version != SCHEMA_VERSION {
        conn.pragma_update(None, "foreign_keys", "OFF")?;
        conn.execute_batch(
            "DROP TABLE IF EXISTS aliases;
             DROP TABLE IF EXISTS memos;
             DROP TABLE IF EXISTS artifacts;
             DROP TABLE IF EXISTS cwd_history;
             DROP TABLE IF EXISTS session_turns;
             DROP TABLE IF EXISTS turns;
             DROP TABLE IF EXISTS request_meta;
             DROP TABLE IF EXISTS blocks;
             DROP TABLE IF EXISTS sessions;",
        )?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sessions (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            name           TEXT,
            started_at     TEXT NOT NULL,
            cwd            TEXT,
            provider       TEXT,
            model          TEXT,
            anchor         INTEGER,
            meta           INTEGER,
            todo_on        INTEGER NOT NULL DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS session_turns (
            session_id    INTEGER NOT NULL REFERENCES sessions(id),
            idx           INTEGER NOT NULL,
            turn_id       INTEGER NOT NULL,
            is_compaction INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (session_id, idx)
        );
        CREATE INDEX IF NOT EXISTS session_turns_by_turn ON session_turns(turn_id);
        CREATE TABLE IF NOT EXISTS turns (
            turn_id            INTEGER PRIMARY KEY AUTOINCREMENT,
            blocks             TEXT NOT NULL DEFAULT '[]',
            stop_reason        TEXT,
            prompt_tokens      INTEGER,
            completion_tokens  INTEGER,
            cached_tokens      INTEGER,
            cache_write_tokens INTEGER
        );
        CREATE TABLE IF NOT EXISTS request_meta (
            request_id  INTEGER PRIMARY KEY AUTOINCREMENT,
            hash        TEXT NOT NULL UNIQUE,
            model           TEXT NOT NULL,
            provider        TEXT NOT NULL,
            system          TEXT NOT NULL,
            tools_json      TEXT NOT NULL,
            baseline_prompt INTEGER
        );
        CREATE TABLE IF NOT EXISTS blocks (
            block_id           INTEGER PRIMARY KEY AUTOINCREMENT,
            type               TEXT NOT NULL,
            content            TEXT,
            prefix             TEXT,
            suffix             TEXT,
            usage_total        INTEGER,
            usage_prompt       INTEGER,
            usage_completion   INTEGER,
            usage_cached       INTEGER,
            usage_cache_write  INTEGER,
            usage_reasoning    INTEGER,
            call_id            TEXT,
            name               TEXT,
            args               TEXT,
            intent             TEXT,
            text               TEXT,
            first              INTEGER,
            ok                 INTEGER,
            result             TEXT,
            memo               TEXT,
            duration_ms        INTEGER,
            ts                 TEXT
        );
        CREATE TABLE IF NOT EXISTS cwd_history (
            session_id INTEGER NOT NULL REFERENCES sessions(id),
            turn_id    INTEGER NOT NULL,
            cwd        TEXT NOT NULL,
            PRIMARY KEY (session_id, turn_id)
        );
        CREATE TABLE IF NOT EXISTS artifacts (
            block_id    INTEGER PRIMARY KEY,
            name        TEXT NOT NULL,
            total_lines INTEGER NOT NULL,
            content     TEXT NOT NULL,
            hash        TEXT NOT NULL,
            reference   TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS artifacts_by_reference ON artifacts(reference);
        CREATE TABLE IF NOT EXISTS memos (
            id      INTEGER PRIMARY KEY AUTOINCREMENT,
            anchor  INTEGER,
            name    TEXT NOT NULL,
            content TEXT NOT NULL,
            ts      TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS memos_by_anchor ON memos(anchor, name, id);
        CREATE INDEX IF NOT EXISTS memos_global ON memos(name, id) WHERE anchor IS NULL;
        CREATE TABLE IF NOT EXISTS aliases (
            id      INTEGER PRIMARY KEY AUTOINCREMENT,
            anchor  INTEGER NOT NULL,
            name    TEXT NOT NULL,
            command TEXT NOT NULL,
            ts      TEXT NOT NULL
        );
        CREATE UNIQUE INDEX IF NOT EXISTS aliases_by_anchor ON aliases(anchor, name);",
    )?;
    Ok(conn)
}
