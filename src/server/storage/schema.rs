//! Storage schema: one authority on the SQLite layout — seven tables, and the connection's three
//! settings (WAL, `synchronous = NORMAL`, a five-second busy timeout). The full spec is in this
//! folder's `README.md`.

use anyhow::{Context, Result};
use rusqlite::Connection;

/// Layout version. On mismatch the whole database is dropped and rebuilt: the project is
/// unreleased, and migration code would be pure burden.
pub const SCHEMA_VERSION: i64 = 8;

/// Open (or create) the database, and make sure its layout is current. The writer owns the result.
pub fn prepare(path: &std::path::Path) -> Result<Connection> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)
            .with_context(|| format!("cannot create data directory {}", dir.display()))?;
    }
    let conn = Connection::open(path)
        .with_context(|| format!("cannot open database {}", path.display()))?;
    // Under WAL a second connection hitting the write lock returns SQLITE_BUSY at once; five
    // seconds is enough to finish a round.
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if version != SCHEMA_VERSION {
        // Old layout goes straight to the bin: wipe, then rebuild.
        conn.pragma_update(None, "foreign_keys", "OFF")?;
        conn.execute_batch(
            "DROP TABLE IF EXISTS session_turns;
             DROP TABLE IF EXISTS turns;
             DROP TABLE IF EXISTS request_meta;
             DROP TABLE IF EXISTS blocks;
             DROP TABLE IF EXISTS artifacts;
             DROP TABLE IF EXISTS cwd_history;
             DROP TABLE IF EXISTS entries;
             DROP TABLE IF EXISTS rounds;
             DROP TABLE IF EXISTS sessions;",
        )?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS sessions (
            id             INTEGER PRIMARY KEY AUTOINCREMENT,
            -- The name written by `/name`. The transcript carries no
            -- name marker: this column is the name's only home.
            name           TEXT,
            started_at     TEXT NOT NULL,
            -- Where the last `/cdp` landed (NULL = never migrated; resume
            -- uses the caller's cwd).
            cwd            TEXT,
            -- Which model this session is on (provider + bare id).
            -- NULL = follow the configured default.
            model_provider TEXT,
            model          TEXT,
            -- Whether the todo tool is on for this session. The list
            -- itself is not stored: it is replayed from this session's
            -- `todo` call arguments (see `README.md`, the tables section).
            todo_on        INTEGER NOT NULL DEFAULT 0
        );
        -- The session's context list: one row per item, idx is the order
        -- (inherited first, own after).
        CREATE TABLE IF NOT EXISTS session_turns (
            session_id    INTEGER NOT NULL REFERENCES sessions(id),
            idx           INTEGER NOT NULL,
            turn_id       INTEGER NOT NULL,
            -- 'own' | 'inherited': whose row this is (a forked prefix
            -- copy writes inherited).
            kind          TEXT NOT NULL,
            -- Whether this row is a compaction round. **It is the read
            -- start** — the session row does not record was-compacted;
            -- the truth lives in the list (and truncating the list on
            -- rewind moves the start back automatically).
            is_compaction INTEGER NOT NULL DEFAULT 0,
            PRIMARY KEY (session_id, idx)
        );
        CREATE INDEX IF NOT EXISTS session_turns_by_turn ON session_turns(turn_id);
        -- Finding the read start means the newest compaction round in the list: a never-compacted session would degrade to a full scan on
        -- the default primary key, so compaction rows get a partial index.
        CREATE INDEX IF NOT EXISTS session_turns_compaction
            ON session_turns(session_id, idx) WHERE is_compaction;
        -- Rounds: the block index is interval ids (one interval per
        -- batch); bytes is the round's block volume.
        CREATE TABLE IF NOT EXISTS turns (
            turn_id           INTEGER PRIMARY KEY AUTOINCREMENT,
            request_id        INTEGER REFERENCES request_meta(request_id),
            blocks            TEXT NOT NULL DEFAULT '[]',
            stop_reason       TEXT,
            bytes             INTEGER NOT NULL DEFAULT 0,
            -- The token columns live and die together: the gateway
            -- either reports usage whole or not at all. Borrowed rows keep
            -- them all NULL (borrow the index, not the bill).
            prompt_tokens     INTEGER,
            completion_tokens INTEGER,
            cached_tokens     INTEGER,
            reasoning_tokens  INTEGER,
            cache_write_tokens INTEGER
        );
        -- Request archive: the conversation-invariant part of one request.
        -- One row per distinct archive (deduplicated by hash).
        CREATE TABLE IF NOT EXISTS request_meta (
            request_id  INTEGER PRIMARY KEY AUTOINCREMENT,
            hash        TEXT NOT NULL UNIQUE,
            model       TEXT NOT NULL,
            provider    TEXT NOT NULL,
            protocol    TEXT NOT NULL,
            base_url    TEXT NOT NULL,
            system      TEXT NOT NULL,
            tools_json  TEXT NOT NULL,
            max_tokens  INTEGER NOT NULL,
            compat_json TEXT
        );
        -- Blocks: one row per message, one column per field. A kind
        -- fills its own columns and leaves the rest NULL; `ts` is when
        -- the block landed.
        CREATE TABLE IF NOT EXISTS blocks (
            block_id         INTEGER PRIMARY KEY AUTOINCREMENT,
            type             TEXT NOT NULL,
            content          TEXT,
            prefix           TEXT,
            suffix           TEXT,
            usage_total      INTEGER,
            usage_prompt     INTEGER,
            usage_completion INTEGER,
            usage_cached     INTEGER,
            usage_cache_write INTEGER,
            usage_reasoning  INTEGER,
            call_id          TEXT,
            name             TEXT,
            args             TEXT,
            intent           TEXT,
            text             TEXT,
            first            INTEGER,
            ok               INTEGER,
            result           TEXT,
            memo             TEXT,
            duration_ms      INTEGER,
            ts               TEXT
        );
        -- Working-directory history: anchored on the **round** (not the
        -- block — that one moves).
        CREATE TABLE IF NOT EXISTS cwd_history (
            session_id INTEGER NOT NULL REFERENCES sessions(id),
            turn_id    INTEGER NOT NULL,
            cwd        TEXT NOT NULL,
            PRIMARY KEY (session_id, turn_id)
        );
        CREATE TABLE IF NOT EXISTS artifacts (
            -- The block that produced it: the key itself, one artifact per
            -- block. The block lands first, so the id is known here.
            block_id    INTEGER PRIMARY KEY,
            name        TEXT NOT NULL,
            total_lines INTEGER NOT NULL,
            content     TEXT NOT NULL
        );",
    )?;
    Ok(conn)
}
