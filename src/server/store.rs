//! Session storage: sessions, blocks, turns, cwd history, artifacts.
//!
//! **A block is the unit.** One row per node as a human sees it (see
//! `crate::grouping`): a user card, an assistant reply, a glued tool exchange.
//! `block_id` is a plain autoincrement (`INTEGER PRIMARY KEY` = rowid), which
//! makes the ordering key three things at once — chronological, unique, and
//! comparable *across* sessions, so a page that spans a fork point sorts
//! correctly with one `ORDER BY`.
//!
//! **A branch is a range, not a chain of rows.** A forked session owns only the
//! blocks it wrote; its transcript is those plus the ancestor prefixes its
//! `fork_block_id`s name. Reading one is a walk over a handful of
//! `(session, upper bound)` segments, each an index range scan — no recursive
//! CTE, and no per-row parent pointer re-deriving a shape storage already knows.
//!
//! **Deleting is truncating.** A session another branch still reads from is not
//! removed: its blocks above the highest fork point any live branch needs are
//! dropped, and the row stays as a tombstone (`deleted_at`) that keeps the
//! `parent_id` chain intact. Nothing is ever re-owned, so no `UPDATE` ever
//! rewrites a range of payload rows.

use anyhow::{Context, Result};
use rusqlite::Connection;

use crate::server::entry::Entry;

/// One block as stored: 1 or 2 entries (a tool exchange is one block).
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub id: i64,
    pub ts: String,
    pub entries: Vec<Entry>,
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
    /// Stored bytes of this branch's own blocks — the picker's size column.
    /// A running total on the row (see `append_in`): the alternative is
    /// `SUM(LENGTH(payload))` over every block on every list, which measured
    /// 66 ms for a few dozen sessions.
    pub bytes: i64,
}

// The part of a request that **cannot** be re-derived from the transcript:
// who we talked to, on which endpoint, under which system prompt, with which
// tool manuals, and how much room the reply had.
//
// Written by the turn runner (the only layer that knows what the request
// actually carried) and persisted with the turn it belongs to.
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
    /// Token ceiling sent with this turn's requests (the configured one;
    /// the per-request value the loop derives from it is recomputable).
    pub max_tokens: u32,
}

/// One stored request header (a row of `turns`).
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
    /// How the turn ended; `None` = died before the gateway answered.
    pub stop_reason: Option<String>,
    /// Block span this turn wrote (`None` when the turn stored no blocks).
    pub first_block: Option<i64>,
    pub last_block: Option<i64>,
}

/// One segment of a branch: `session`'s blocks up to and including `upper`.
///
/// A branch is the concatenation of its own segment and every ancestor's, which
/// is why the walk needs no recursion and why a page can be read segment by
/// segment with a plain index range scan.
struct Seg {
    session: i64,
    upper: i64,
    /// Does this session inherit a prefix? (Its block-0 cwd row is then a copy
    /// of its parent's last one, not a migration of its own.)
    has_parent: bool,
}

/// A stored block's raw columns, before decoding.
struct RawBlock {
    id: i64,
    ts: String,
    kind: String,
    payload: String,
    kind2: Option<String>,
    payload2: Option<String>,
}

impl RawBlock {
    /// Decode to entries. `false` = at least one half had an unknown kind
    /// (forward compatibility: skip what we cannot read, keep what we can).
    fn into_entries(self) -> (Vec<Entry>, bool) {
        let mut out = Vec::with_capacity(2);
        let mut ok = true;
        match Entry::from_payload(&self.kind, &self.payload) {
            Some(e) => out.push(e),
            None => ok = false,
        }
        if let (Some(k), Some(p)) = (self.kind2.as_deref(), self.payload2.as_deref()) {
            match Entry::from_payload(k, p) {
                Some(e) => out.push(e),
                None => ok = false,
            }
        }
        (out, ok)
    }

    /// Decode to a block **with its id** — the shape a front end windows on.
    fn into_block(self) -> Block {
        let id = self.id;
        let ts = self.ts.clone();
        Block {
            id,
            ts,
            entries: self.into_entries().0,
        }
    }
}

const BLOCK_COLS: &str = "block_id, ts, kind, payload, kind2, payload2";

fn round_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RoundRow> {
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
        first_block: r.get(9)?,
        last_block: r.get(10)?,
    })
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
        // 忙等：WAL 下多个连接（daemon 写 + 一次性命令读、测试并行）撞上写锁时
        // 立即返回 SQLITE_BUSY 会让"数据库被锁"变成随机失败。5 秒足够写完一轮。
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        // 别的布局**直接扔掉**：项目未发布、只有一个用户、没有要保留的历史，
        // 迁移代码只会变成负担。`user_version` 对不上就把表全删了重建——
        // 开发期改 schema 只需把这个数字加一。
        const SCHEMA_VERSION: i64 = 1;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version != SCHEMA_VERSION {
            // Foreign keys are on (the bundled SQLite turns them on by
            // default), so drop children before parents and lift the
            // enforcement for the reset itself.
            conn.pragma_update(None, "foreign_keys", "OFF")?;
            conn.execute_batch(
                "DROP TABLE IF EXISTS blocks;
                 DROP TABLE IF EXISTS turns;
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
                id            INTEGER PRIMARY KEY,
                -- 对外身份（导出/合并用）。整数 id 仍是内部键：TEXT 主键在
                -- SQLite 里是二级索引，比较的是 36 字节而不是 8。
                uuid          TEXT NOT NULL UNIQUE,
                name          TEXT,
                started_at    TEXT NOT NULL,
                cwd           TEXT,
                -- 分支：parent_id 非空 = 从父会话的 fork_block_id 分叉出来
                -- （继承 **<= fork_block_id** 的前缀）。tip_block_id = 自己
                -- 最后一块（NULL = 还没写过自己的块）。
                parent_id     INTEGER REFERENCES sessions(id),
                fork_block_id INTEGER,
                tip_block_id  INTEGER,
                -- 自己块的载荷字节数（列表页的体积列，写入时累加）。
                bytes         INTEGER NOT NULL DEFAULT 0,
                -- 墓碑：内容被截断、行被留下，只为让子分支还能沿链读到前缀。
                deleted_at    TEXT
            );
            CREATE TABLE IF NOT EXISTS blocks (
                -- 全局自增 = 时序键：跨会话可比、唯一、免费。
                block_id   INTEGER PRIMARY KEY,
                session_id INTEGER NOT NULL REFERENCES sessions(id),
                ts         TEXT NOT NULL,
                kind       TEXT NOT NULL,
                payload    TEXT NOT NULL,
                -- 工具往返的第二半（一块两半）。只有 tool_request+tool_result
                -- 会粘成一块，所以 kind2 只会是 tool_result。
                kind2      TEXT,
                payload2   TEXT
            );
            CREATE INDEX IF NOT EXISTS blocks_by_session ON blocks(session_id, block_id);
            CREATE INDEX IF NOT EXISTS blocks_by_kind ON blocks(session_id, kind, block_id);
            CREATE TABLE IF NOT EXISTS cwd_history (
                session_id INTEGER NOT NULL REFERENCES sessions(id),
                -- 换目录时所在的那一块（0 = 会话原点）。
                block_id   INTEGER NOT NULL,
                -- 那一块之前**已经有多少条条目**（分支根算起，绝对位置）。
                -- 写的时候算一次：读它的 `context` 工具要的是条目位置，而只有
                -- 存储知道一块是一条还是两条——放在读路径上就是每次开回合扫
                -- 一遍全库（32k 块实测 16 ms）。
                entry_index INTEGER NOT NULL,
                ts         TEXT NOT NULL,
                cwd        TEXT NOT NULL,
                PRIMARY KEY (session_id, block_id)
            );
            CREATE TABLE IF NOT EXISTS artifacts (
                id          INTEGER PRIMARY KEY,
                session_id  INTEGER NOT NULL REFERENCES sessions(id),
                -- 产生它的回合的第一块：回合是分叉的边界，所以「跟着哪一块」
                -- 等价于「跟着哪一回合」。NULL = 还没随回合落盘。
                block_id    INTEGER,
                name        TEXT NOT NULL,
                total_lines INTEGER NOT NULL,
                content     TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS artifacts_by_block ON artifacts(session_id, block_id);
            CREATE TABLE IF NOT EXISTS turns (
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
                first_block INTEGER,
                last_block  INTEGER,
                PRIMARY KEY (session_id, seq)
            );",
        )?;
        Ok(Self { conn })
    }

    // Create a session, returning its id. `started_at` is written when
    // the first turn starts; `cwd` records the initial working directory.
    pub fn create_session(&mut self, started_at: &str, cwd: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO sessions (uuid, started_at, cwd) VALUES (?1, ?2, ?3)",
            rusqlite::params![new_uuid(), started_at, cwd],
        )?;
        let id = self.conn.last_insert_rowid();
        // The initial directory is itself a cwd history entry (block 0 = origin)
        self.record_cwd(id, cwd)?;
        Ok(id)
    }

    /// Fork `parent_id` at `fork_block_id` (inclusive): the new session inherits
    /// every block up to and including it and owns none of its own yet.
    ///
    /// Nothing is copied. A fork is one row, so forking a 30 000-block
    /// conversation costs the same as forking a two-message one.
    pub fn fork_session(&mut self, parent_id: i64, fork_block_id: i64) -> Result<i64> {
        anyhow::ensure!(
            self.block_on_branch(parent_id, fork_block_id)?,
            "会话 {parent_id} 的分支上没有块 {fork_block_id}"
        );
        let cwd = self.session(parent_id)?.cwd.unwrap_or_default();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO sessions (uuid, started_at, cwd, parent_id, fork_block_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![new_uuid(), now_stamp(), cwd, parent_id, fork_block_id],
        )?;
        let id = tx.last_insert_rowid();
        tx.execute(
            "INSERT INTO cwd_history (session_id, block_id, entry_index, ts, cwd)
             VALUES (?1, 0, 0, ?2, ?3)",
            rusqlite::params![id, now_stamp(), cwd],
        )?;
        tx.commit()?;
        Ok(id)
    }

    /// Is `block_id` part of `session_id`'s branch (its own blocks or an
    /// ancestor prefix it inherits)?
    fn block_on_branch(&self, session_id: i64, block_id: i64) -> Result<bool> {
        for seg in self.chain(session_id, None)? {
            let n: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM blocks
                  WHERE session_id = ?1 AND block_id = ?2 AND block_id <= ?3",
                rusqlite::params![seg.session, block_id, seg.upper],
                |r| r.get(0),
            )?;
            if n > 0 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    // /cdp: permanent migration — update sessions.cwd and append a history row
    // keyed by the block the session currently sits on. Every migration is
    // recorded so resume can land on any of them.
    pub fn record_cwd(&mut self, session_id: i64, cwd: &str) -> Result<()> {
        let block = self.tip(session_id)?.unwrap_or(0);
        // The migration lands **after** the tip block's entries, so count up to
        // the tip inclusive (`block + 1` is "everything ≤ block" on this
        // branch, and the next real block id is always greater).
        let entry_index = self.entries_before(session_id, block + 1)?;
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO cwd_history (session_id, block_id, entry_index, ts, cwd)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(session_id, block_id) DO UPDATE SET
                 entry_index = excluded.entry_index, cwd = excluded.cwd, ts = excluded.ts",
            rusqlite::params![session_id, block, entry_index, now_stamp(), cwd],
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

    // Append a whole turn of entries in one transaction, one block per grouping
    // node. All-or-nothing: the turn is either fully stored or not at all.
    pub fn append(&mut self, session_id: i64, entries: &[Entry]) -> Result<()> {
        let tx = self.conn.transaction()?;
        Self::append_in(&tx, session_id, entries)?;
        tx.commit()?;
        Ok(())
    }

    // The block-append body, on a caller-owned transaction: everything that
    // writes blocks shares this so "advance the tip / add the bytes" can never
    // diverge between the turn path and the marker path.
    //
    // Returns the (first, last) allocated block ids — the span a turn row
    // records.
    //
    // Blocks are grouped **within this batch** (`grouping::chunks`), never
    // across batches: a turn's entries are staged in memory and written in one
    // call, so the last block of a batch is complete by construction and no
    // stored row is ever mutated afterwards.
    fn append_in(
        tx: &rusqlite::Transaction<'_>,
        session_id: i64,
        entries: &[Entry],
    ) -> Result<(Option<i64>, Option<i64>)> {
        if entries.is_empty() {
            return Ok((None, None));
        }
        let ts = now_stamp();
        let mut first = None;
        let mut last = None;
        let mut bytes = 0i64;
        for r in crate::grouping::chunks(entries) {
            let (kind, payload) = entries[r.start].to_payload();
            let (kind2, payload2) = if r.end > r.start + 1 {
                let (k, p) = entries[r.start + 1].to_payload();
                (Some(k), Some(p))
            } else {
                (None, None)
            };
            bytes += payload.len() as i64 + payload2.as_ref().map_or(0, |p| p.len() as i64);
            tx.execute(
                "INSERT INTO blocks (session_id, ts, kind, payload, kind2, payload2)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![session_id, ts, kind, payload, kind2, payload2],
            )?;
            let id = tx.last_insert_rowid();
            first.get_or_insert(id);
            last = Some(id);
        }
        tx.execute(
            "UPDATE sessions SET tip_block_id = ?1, bytes = bytes + ?2 WHERE id = ?3",
            rusqlite::params![last, bytes, session_id],
        )?;
        Ok((first, last))
    }

    // Persist one turn **and its request header** in a single transaction.
    //
    // The header is what makes a stored conversation byte-reproducible from the
    // database alone: the system prompt, the tool manuals, the model id, the
    // endpoint and the token ceiling that the request actually carried. None of
    // it may be re-derived from local config/profile at read time — the DB is
    // the only source, so a different machine, a different UI or an edited
    // config still replays the exact bytes that went to the gateway.
    //
    // `stop` is the turn's end reason (`StopReason::as_str`), or `None` for a
    // turn that died before the gateway answered (mid-flight error): the
    // partial reply is still stored, and a NULL stop_reason is the honest
    // record of "we never got a final stop".
    pub fn append_round(
        &mut self,
        session_id: i64,
        entries: &[Entry],
        meta: &RoundMeta,
        stop: Option<&str>,
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        let (first_block, last_block) = Self::append_in(&tx, session_id, entries)?;
        // 巨物归属：本回合之前落的 artifact（block_id 还空着）挂到本回合第一块。
        // 回合是分叉的边界，所以"跟着哪一块"等价于"跟着哪一回合"——截断时按
        // 区间删就既删不掉还被读着的巨物，也不会留下没人引用的。
        if let Some(f) = first_block {
            tx.execute(
                "UPDATE artifacts SET block_id = ?1 WHERE session_id = ?2 AND block_id IS NULL",
                rusqlite::params![f, session_id],
            )?;
        }
        let seq: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) + 1 FROM turns WHERE session_id = ?1",
                [session_id],
                |r| r.get(0),
            )
            .context("failed to query max turn seq")?;
        tx.execute(
            "INSERT INTO turns (session_id, seq, ts, model, protocol, base_url, system,
                                tools_json, max_tokens, stop_reason, first_block, last_block)
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
                first_block,
                last_block
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    // Every stored request header of a session, oldest first.
    pub fn rounds(&self, session_id: i64) -> Result<Vec<RoundRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, ts, model, protocol, base_url, system, tools_json, max_tokens,
                    stop_reason, first_block, last_block
             FROM turns WHERE session_id = ?1 ORDER BY seq",
        )?;
        let rows = stmt.query_map([session_id], round_from_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    // One stored request header, or None when the turn is not stored.
    pub fn round(&self, session_id: i64, seq: i64) -> Result<Option<RoundRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT seq, ts, model, protocol, base_url, system, tools_json, max_tokens,
                    stop_reason, first_block, last_block
             FROM turns WHERE session_id = ?1 AND seq = ?2",
        )?;
        let mut rows = stmt.query_map(rusqlite::params![session_id, seq], round_from_row)?;
        match rows.next() {
            Some(row) => Ok(Some(row?)),
            None => Ok(None),
        }
    }

    // Load a session's branch (root → tip): its own blocks plus the ancestor
    // prefixes it inherits.
    pub fn load_entries(&self, session_id: i64) -> Result<Vec<Entry>> {
        Ok(self.load_branch(session_id, None)?.0)
    }

    // Load a branch as (readable entries, ids of blocks that could not be
    // parsed), optionally clamped to a tip: replaying an older turn must
    // rebuild the conversation that was live **then**, not today's branch.
    //
    // The unreadable list exists because the two callers want opposite things:
    // a UI must degrade gracefully (skip a block it does not understand,
    // forward compatibility), while **replay must not** — silently dropping one
    // row changes the request bytes, which is exactly what the stored turn is
    // supposed to make impossible. So the information is returned instead of
    // discarded here.
    pub fn load_branch(&self, session_id: i64, tip: Option<i64>) -> Result<(Vec<Entry>, Vec<i64>)> {
        let mut segs = self.chain(session_id, tip)?;
        segs.reverse(); // root-first: an ancestor's prefix comes before ours
        let mut out = Vec::new();
        let mut missing = Vec::new();
        for seg in segs {
            let sql = format!(
                "SELECT {BLOCK_COLS} FROM blocks
                  WHERE session_id = ?1 AND block_id <= ?2 ORDER BY block_id"
            );
            for raw in self.read_raw(&sql, vec![seg.session.into(), seg.upper.into()])? {
                let id = raw.id;
                let (entries, ok) = raw.into_entries();
                if !ok {
                    missing.push(id);
                }
                out.extend(entries);
            }
        }
        Ok((out, missing))
    }

    /// The branch's **last** `limit` blocks (ascending), plus "is there
    /// anything above the oldest one".
    ///
    /// 冷启动第一帧只要一屏：32k 块脏库上整读要 261 ms（读 81 + 解析 81 +
    /// 路径走查 ~100），而真正为第一帧服务的是最后那几十块。这条只读尾巴，
    /// 而且**从叶子段往回走**——每段一次索引区间扫，分叉深度是几就几跳。
    pub fn load_tail(&self, session_id: i64, limit: usize) -> Result<(Vec<Block>, bool)> {
        self.page(session_id, None, limit)
    }

    /// `limit` blocks older than `before_block` (ascending) — the page before
    /// [`Self::load_tail`]. The **caller names the boundary id**, so the front
    /// end's window needs no server-side cursor: it asks for what it is about
    /// to drop, not for "the next page".
    pub fn load_before(
        &self,
        session_id: i64,
        before_block: i64,
        limit: usize,
    ) -> Result<(Vec<Block>, bool)> {
        self.page(session_id, Some(before_block), limit)
    }

    /// `limit` blocks **newer** than `after_block` (ascending) — what a front
    /// end re-reads after scrolling back down into a region it had evicted.
    ///
    /// Mirror of [`Self::load_before`]: the chain is walked root-first here
    /// (ids grow with time within a branch), each segment contributing its
    /// blocks above the cursor, clamped by that segment's upper bound so a
    /// fork's parent prefix never leaks past the fork point.
    pub fn load_after(
        &self,
        session_id: i64,
        after_block: i64,
        limit: usize,
    ) -> Result<Vec<Block>> {
        let mut chain = self.chain(session_id, None)?; // leaf-first
        chain.reverse(); // root-first: ids ascend along the walk
        let mut acc: Vec<RawBlock> = Vec::with_capacity(limit);
        for seg in chain {
            if acc.len() >= limit {
                break;
            }
            let need = (limit - acc.len()) as i64;
            let sql = format!(
                "SELECT {BLOCK_COLS} FROM blocks
                  WHERE session_id = ?1 AND block_id > ?2 AND block_id <= ?3
                  ORDER BY block_id ASC LIMIT ?4"
            );
            let mut rows = self.read_raw(
                &sql,
                vec![seg.session.into(), after_block.into(), seg.upper.into(), need.into()],
            )?;
            acc.append(&mut rows);
        }
        Ok(acc.into_iter().map(RawBlock::into_block).collect())
    }

    /// 共用的分页读：从叶子段往回走，每段"降序 `need` 块"，凑够 `limit + 1`
    /// 就停（多出来那一块只用来判"上面还有没有"）。段取空了就跳到父段，上界是
    /// 子会话的 `fork_block_id`。游标是**块**，所以一页永远不会把一次工具往返
    /// 劈成两半。
    fn page(
        &self,
        session_id: i64,
        before: Option<i64>,
        limit: usize,
    ) -> Result<(Vec<Block>, bool)> {
        let chain = self.chain(session_id, None)?; // leaf-first
        let want = limit + 1;
        let mut acc: Vec<RawBlock> = Vec::with_capacity(want);
        for seg in chain {
            if acc.len() >= want {
                break;
            }
            let need = (want - acc.len()) as i64;
            let sql = format!(
                "SELECT {BLOCK_COLS} FROM blocks
                  WHERE session_id = ?1 AND block_id <= ?2{} ORDER BY block_id DESC LIMIT ?{}",
                if before.is_some() { " AND block_id < ?3" } else { "" },
                if before.is_some() { 4 } else { 3 },
            );
            let mut params = vec![seg.session.into(), seg.upper.into()];
            if let Some(b) = before {
                params.push(b.into());
            }
            params.push(need.into());
            let mut rows = self.read_raw(&sql, params)?;
            acc.append(&mut rows);
        }
        let has_more = acc.len() > limit;
        acc.truncate(limit);
        acc.reverse();
        Ok((acc.into_iter().map(RawBlock::into_block).collect(), has_more))
    }

    /// The segments of a branch, leaf-first: `[(session, upper bound), …]`.
    ///
    /// `tip` clamps the session's own segment (replay of an older turn); the
    /// ancestors' bounds come from the fork points and never move.
    fn chain(&self, session_id: i64, tip: Option<i64>) -> Result<Vec<Seg>> {
        let mut out = Vec::new();
        let mut cur = Some(session_id);
        let mut upper = tip;
        let mut first = true;
        while let Some(sid) = cur {
            let (parent, fork, own_tip) = self.branch_row(sid)?;
            // -1 = "no blocks at all": a session that has not written its own
            // first block yet (a fresh fork) still has its parent's prefix.
            let bound = if first { upper.or(own_tip) } else { upper }.unwrap_or(-1);
            out.push(Seg {
                session: sid,
                upper: bound,
                has_parent: parent.is_some(),
            });
            upper = fork;
            cur = parent;
            first = false;
        }
        Ok(out)
    }

    fn branch_row(&self, id: i64) -> Result<(Option<i64>, Option<i64>, Option<i64>)> {
        self.conn
            .query_row(
                "SELECT parent_id, fork_block_id, tip_block_id FROM sessions WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .with_context(|| format!("会话 {id} 不存在"))
    }

    fn read_raw(&self, sql: &str, params: Vec<rusqlite::types::Value>) -> Result<Vec<RawBlock>> {
        let mut stmt = self.conn.prepare_cached(sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params), |r| {
            Ok(RawBlock {
                id: r.get(0)?,
                ts: r.get(1)?,
                kind: r.get(2)?,
                payload: r.get(3)?,
                kind2: r.get(4)?,
                payload2: r.get(5)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// The blocks this session **owns**, oldest first (tests and benches).
    #[doc(hidden)]
    pub fn stored_blocks(&self, session_id: i64) -> Result<Vec<Block>> {
        let sql = format!("SELECT {BLOCK_COLS} FROM blocks WHERE session_id = ?1 ORDER BY block_id");
        Ok(self
            .read_raw(&sql, vec![session_id.into()])?
            .into_iter()
            .map(RawBlock::into_block)
            .collect())
    }

    // Current tip block of a session's **own** blocks (None = a fresh fork).
    pub fn tip(&self, session_id: i64) -> Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT tip_block_id FROM sessions WHERE id = ?1",
                [session_id],
                |r| r.get(0),
            )
            .map_err(Into::into)
    }

    // Effective session name: the nearest `name` marker looking back along the
    // branch (pi's session_info semantic — a branch inherits the name it was
    // created under, siblings never see it). Falls back to the plain
    // `sessions.name` column.
    //
    // One indexed row per segment, not the whole transcript: this runs on every
    // resume, and materializing every block just to find a marker would put an
    // O(n) read back on the hot path by itself. (`kind` alone is enough: the
    // only glued block is a tool exchange, so `kind2` is never `name`.)
    pub fn effective_name(&self, session_id: i64) -> Result<Option<String>> {
        for seg in self.chain(session_id, None)? {
            let mut stmt = self.conn.prepare_cached(
                "SELECT payload FROM blocks
                  WHERE session_id = ?1 AND kind = 'name' AND block_id <= ?2
                  ORDER BY block_id DESC LIMIT 1",
            )?;
            let mut rows = stmt.query(rusqlite::params![seg.session, seg.upper])?;
            if let Some(row) = rows.next()? {
                let payload: String = row.get(0)?;
                if let Some(Entry::Name { name }) = Entry::from_payload("name", &payload) {
                    return Ok(Some(name));
                }
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

    // List all live sessions, newest first. Used by the resume picker.
    pub fn list_sessions(&self) -> Result<Vec<SessionMeta>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, started_at, cwd FROM sessions
              WHERE deleted_at IS NULL ORDER BY id DESC",
        )?;
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
    /// Everything the picker draws comes back in **one** query: the first user
    /// message (preview line + fallback display name) and the stored byte count
    /// (a running total on the row, not a scan of every block).
    pub fn list_session_rows(&self, under: Option<&std::path::Path>) -> Result<Vec<SessionRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, s.name, s.started_at, s.cwd,
                    (SELECT b.payload FROM blocks b
                      WHERE b.session_id = s.id AND b.kind = 'user'
                      ORDER BY b.block_id LIMIT 1),
                    s.bytes
             FROM sessions s
             WHERE s.deleted_at IS NULL
               AND (?1 IS NULL
                    OR s.id IN (SELECT DISTINCT session_id FROM cwd_history WHERE cwd = ?1))
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

    /// Bench/diagnostic harness getter (doc-hidden, not API): the stored
    /// `(kind, payload)` rows for a session **without parsing them**, so a
    /// harness can tell SQLite read cost apart from JSON parse cost.
    #[doc(hidden)]
    pub fn bench_raw_rows(&self, session_id: i64) -> Result<Vec<(String, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT kind, payload FROM blocks WHERE session_id = ?1 ORDER BY block_id")?;
        let rows = stmt.query_map([session_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
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

    /// The session's outward identity (export/merge). Not shown anywhere yet:
    /// the integer id is what every read path and the front end use.
    pub fn session_uuid(&self, session_id: i64) -> Result<Option<String>> {
        use rusqlite::OptionalExtension;
        self.conn
            .query_row("SELECT uuid FROM sessions WHERE id = ?1", [session_id], |r| {
                r.get(0)
            })
            .optional()
            .map_err(Into::into)
    }

    // The session cwd migration history (ascending). block 0 = session origin.
    pub fn cwd_history(&self, session_id: i64) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT block_id, cwd FROM cwd_history WHERE session_id = ?1 ORDER BY block_id",
        )?;
        let rows = stmt.query_map([session_id], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// cwd migrations as `(entry index, path)`, positions being indices into
    /// [`Self::load_entries`] — the `context` tool annotates windows against
    /// that vector.
    ///
    /// The index is stored on the row (`entry_index`), computed once when the
    /// migration happens: deriving it here would mean counting the entries of
    /// every preceding block on each turn start (16 ms at 32 000 blocks, and it
    /// grows with the conversation). Indices are root-relative, so a fork reads
    /// its ancestors' rows unchanged.
    pub fn cwd_trail(&self, session_id: i64) -> Result<Vec<(i64, String)>> {
        let chain = self.chain(session_id, None)?; // leaf-first
        let mut per_seg: Vec<Vec<(i64, String)>> = Vec::with_capacity(chain.len());
        for seg in chain {
            // A fork's own origin row (block 0) restates its parent's current
            // directory; the parent's row already carries the right position.
            let first = i64::from(seg.has_parent);
            let mut stmt = self.conn.prepare_cached(
                "SELECT entry_index, cwd FROM cwd_history
                  WHERE session_id = ?1 AND block_id <= ?2 AND block_id >= ?3
                  ORDER BY block_id",
            )?;
            let it = stmt.query_map(rusqlite::params![seg.session, seg.upper, first], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
            per_seg.push(it.collect::<std::result::Result<Vec<_>, _>>()?);
        }
        per_seg.reverse(); // root-first, so the trail reads in transcript order
        Ok(per_seg.into_iter().flatten().collect())
    }

    /// How many entries precede `block` on this branch (root-relative).
    fn entries_before(&self, session_id: i64, block: i64) -> Result<i64> {
        let mut segs = self.chain(session_id, None)?;
        segs.reverse(); // root-first: counts accumulate in transcript order
        let mut base = 0i64;
        for seg in segs {
            // The whole segment, or everything before `block` inside it.
            let (le, lt) = if block > seg.upper {
                (seg.upper, i64::MAX)
            } else {
                (seg.upper, block)
            };
            let n: i64 = self.conn.query_row(
                "SELECT COALESCE(SUM(1 + (kind2 IS NOT NULL)), 0) FROM blocks
                  WHERE session_id = ?1 AND block_id <= ?2 AND block_id < ?3",
                rusqlite::params![seg.session, le, lt],
                |r| r.get(0),
            )?;
            base += n;
            if block <= seg.upper {
                break;
            }
        }
        Ok(base)
    }

    // ---- artifacts: oversized tool outputs stored out-of-band ----
    //
    // A tool result above the threshold is stored here whole; the context
    // gets a one-line placeholder with the artifact id, and the model pulls
    // content back through `#id` virtual files in bash. An artifact belongs to
    // the turn that produced it (its `block_id` is that turn's first block), so
    // truncating a range takes its artifacts with it.

    /// Store an oversized tool output; returns its artifact id. The owning
    /// block is filled in when the turn is persisted (`append_round`).
    pub fn put_artifact(&mut self, session_id: i64, name: &str, content: &str) -> Result<i64> {
        let lines = content.lines().count() as i64;
        self.conn.execute(
            "INSERT INTO artifacts (session_id, name, total_lines, content) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![session_id, name, lines, content],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Fetch one artifact (whole content + metadata).
    ///
    /// **Not** scoped to a session: a fork reads blocks inherited from its
    /// parent, and their `#N` references point at the parent's artifacts. An id
    /// the branch never referenced stays readable — the model can already read
    /// any file through bash, so a session filter here never bought isolation,
    /// only broken references.
    pub fn get_artifact(&self, id: i64) -> Result<Option<(String, i64, String)>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, total_lines, content FROM artifacts WHERE id = ?1")?;
        let mut rows = stmt.query_map([id], |r| {
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

    /// Delete a session.
    ///
    /// **Nobody reads it** (no live branch needs its blocks): the whole subtree
    /// is unreachable, so rows and blocks both go.
    ///
    /// **Somebody still does** (a fork inherits its prefix): the blocks above
    /// the highest fork point any live branch needs are dropped, and the row
    /// stays as a tombstone with `deleted_at` set — a child's `parent_id` must
    /// keep resolving, and the prefix it reads must keep existing. Returns
    /// whether the session existed.
    pub fn delete_session(&mut self, session_id: i64) -> Result<bool> {
        let exists: bool = self
            .conn
            .query_row("SELECT COUNT(*) FROM sessions WHERE id = ?1", [session_id], |r| {
                r.get::<_, i64>(0)
            })
            .map(|n| n > 0)?;
        if !exists {
            return Ok(false);
        }
        match self.keep_of(session_id)? {
            Some(keep) => {
                let tx = self.conn.transaction()?;
                tx.execute(
                    "DELETE FROM blocks WHERE session_id = ?1 AND block_id > ?2",
                    rusqlite::params![session_id, keep],
                )?;
                tx.execute(
                    "DELETE FROM artifacts WHERE session_id = ?1
                       AND (block_id IS NULL OR block_id > ?2)",
                    rusqlite::params![session_id, keep],
                )?;
                tx.execute(
                    "DELETE FROM turns WHERE session_id = ?1 AND first_block > ?2",
                    rusqlite::params![session_id, keep],
                )?;
                tx.execute(
                    "UPDATE sessions
                        SET deleted_at = ?1,
                            tip_block_id = (SELECT MAX(block_id) FROM blocks
                                             WHERE session_id = ?2),
                            bytes = (SELECT COALESCE(SUM(LENGTH(payload)
                                        + COALESCE(LENGTH(payload2), 0)), 0)
                                       FROM blocks WHERE session_id = ?2)
                      WHERE id = ?2",
                    rusqlite::params![now_stamp(), session_id],
                )?;
                tx.commit()?;
            }
            None => {
                // Read the subtree **before** opening the write transaction
                // (the transaction borrows the connection), and delete it
                // deepest-first: a child row references its parent.
                let mut ids = self.subtree(session_id)?;
                ids.reverse();
                let tx = self.conn.transaction()?;
                for id in ids {
                    for table in ["blocks", "cwd_history", "artifacts", "turns"] {
                        tx.execute(&format!("DELETE FROM {table} WHERE session_id = ?1"), [id])?;
                    }
                    tx.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
                }
                tx.commit()?;
            }
        }
        Ok(true)
    }

    /// The highest block id a **live branch still needs** from this session, or
    /// `None` when nothing does.
    ///
    /// A child counts when it is alive, or when it is itself a tombstone a
    /// grandchild still reads through — otherwise deleting a middle branch
    /// would strand its grandchild's prefix. A fork point may sit in an
    /// ancestor's blocks (forking at an older message), which is exactly why the
    /// answer is a bound and not "my own tip".
    fn keep_of(&self, session_id: i64) -> Result<Option<i64>> {
        let children: Vec<(i64, Option<i64>, Option<String>)> = {
            let mut stmt = self.conn.prepare_cached(
                "SELECT id, fork_block_id, deleted_at FROM sessions WHERE parent_id = ?1",
            )?;
            let rows = stmt.query_map([session_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut keep: Option<i64> = None;
        for (child, fork, deleted) in children {
            let needed = deleted.is_none() || self.keep_of(child)?.is_some();
            if needed
                && let Some(f) = fork
            {
                keep = Some(keep.map_or(f, |k: i64| k.max(f)));
            }
        }
        Ok(keep)
    }

    /// A session and every descendant (fork chains are a handful of rows deep).
    fn subtree(&self, root: i64) -> Result<Vec<i64>> {
        let mut out = vec![root];
        let mut i = 0;
        while i < out.len() {
            let kids: Vec<i64> = {
                let mut stmt = self
                    .conn
                    .prepare_cached("SELECT id FROM sessions WHERE parent_id = ?1")?;
                let rows = stmt.query_map([out[i]], |r| r.get(0))?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };
            out.extend(kids);
            i += 1;
        }
        Ok(out)
    }
}

/// Random v4-shaped UUID.
///
/// `/dev/urandom` when it is there; otherwise a time+pid mix, so a machine
/// without it degrades to "unique enough locally" instead of refusing to create
/// a session.
fn new_uuid() -> String {
    let mut b = [0u8; 16];
    let ok = std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut b))
        .is_ok();
    if !ok {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let mix = nanos ^ ((std::process::id() as u128) << 64);
        b.copy_from_slice(&mix.to_le_bytes());
    }
    b[6] = (b[6] & 0x0f) | 0x40; // version 4
    b[8] = (b[8] & 0x3f) | 0x80; // RFC 4122 variant
    let hex: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
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
/// rule as `load_branch`).
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
    use std::path::Path;

    fn mem_store() -> Store {
        Store::open(Path::new(":memory:")).unwrap()
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

    fn user(t: &str) -> Entry {
        Entry::User { content: t.into() }
    }
    fn assistant(t: &str) -> Entry {
        Entry::Assistant {
            content: t.into(),
            usage: None,
        }
    }
    fn req(call: &str) -> Entry {
        Entry::ToolRequest {
            call_id: call.into(),
            name: "bash".into(),
            args: "{}".into(),
            intent: String::new(),
            text: String::new(),
            first: true,
        }
    }
    fn res(call: &str) -> Entry {
        Entry::ToolResult {
            call_id: call.into(),
            name: "bash".into(),
            ok: true,
            result: "ok".into(),
            details: None,
            duration_ms: 0,
        }
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

    fn block_texts(bs: &[Block]) -> Vec<String> {
        let es: Vec<Entry> = bs.iter().flat_map(|b| b.entries.clone()).collect();
        page_texts(&es)
    }

    fn page_texts(es: &[Entry]) -> Vec<String> {
        es.iter()
            .map(|e| match e {
                Entry::User { content } => content.clone(),
                _ => String::new(),
            })
            .collect()
    }

    // ---- 存储形状：块 ----

    /// 一次工具往返落成**一行**（块 = 渲染单元 = 存储单元），而且同一批块
    /// 共享一个时间戳——时间戳定不了序，所以顺序键只能是 block_id。
    #[test]
    fn a_tool_exchange_is_one_block() {
        let mut s = mem_store();
        let id = s.create_session("t", "/tmp").unwrap();
        s.append(id, &[user("a"), req("c1"), res("c1"), assistant("b")])
            .unwrap();
        let blocks = s.stored_blocks(id).unwrap();
        assert_eq!(blocks.len(), 3, "user / 工具往返 / assistant");
        assert_eq!(blocks[1].entries.len(), 2);
        assert_eq!(blocks[0].ts, blocks[1].ts, "同一批块共享时间戳");
    }

    /// 请求没有结果时不成对：它就是自己一块，上下文重建会按"悬空调用"丢掉它。
    #[test]
    fn a_dangling_request_is_its_own_block() {
        let mut s = mem_store();
        let id = s.create_session("t", "/tmp").unwrap();
        s.append(id, &[user("a"), req("c1"), assistant("b")])
            .unwrap();
        let blocks = s.stored_blocks(id).unwrap();
        assert_eq!(blocks.len(), 3);
        assert!(blocks.iter().all(|b| b.entries.len() == 1));
    }

    /// 块号全局自增：分叉写的新块必然大于它继承的前缀，所以一条 `ORDER BY`
    /// 就能把"父前缀 + 自己的块"排对。
    #[test]
    fn block_ids_are_global_across_branches() {
        let mut s = mem_store();
        let a = s.create_session("t", "/tmp").unwrap();
        s.append(a, &[user("a1"), user("a2"), user("a3")]).unwrap();
        let tip = s.tip(a).unwrap().unwrap();
        let b = s.fork_session(a, tip).unwrap();
        s.append(b, &[user("b1")]).unwrap();
        let own_b = s.stored_blocks(b).unwrap();
        assert_eq!(own_b.len(), 1);
        assert!(own_b[0].id > tip, "自己的块必须排在前缀之后");
        assert_eq!(s.tip(b).unwrap(), Some(own_b[0].id));
        assert_eq!(s.tip(a).unwrap(), Some(tip), "分叉不动父的 tip");
    }

    #[test]
    fn uuid_is_unique_per_session() {
        let mut s = mem_store();
        let a = s.create_session("t", "/tmp").unwrap();
        let b = s.create_session("t", "/tmp").unwrap();
        let (ua, ub) = (
            s.session_uuid(a).unwrap().unwrap(),
            s.session_uuid(b).unwrap().unwrap(),
        );
        assert_ne!(ua, ub);
        assert_eq!(ua.len(), 36, "v4 形状");
    }

    #[test]
    fn append_and_load_round_trip() {
        let mut s = mem_store();
        let id = s.create_session("2026-09-22 14:30:05", "/tmp").unwrap();
        let entries = vec![
            user("你好\n世界"),
            req("c1"),
            res("c1"),
            assistant("改好了"),
        ];
        s.append(id, &entries).unwrap();
        let back = s.load_entries(id).unwrap();
        assert_eq!(back, entries, "round-trip must be byte-identical");
    }

    #[test]
    fn unknown_kind_is_skipped_not_fatal() {
        let mut s = mem_store();
        let id = s.create_session("t", "/tmp").unwrap();
        s.append(id, &[user("x")]).unwrap();
        // Insert an unknown kind by hand (simulating a future version's write)
        s.conn
            .execute(
                "INSERT INTO blocks (session_id, ts, kind, payload) VALUES (?1, 't', 'brand_new_kind', '{}')",
                [id],
            )
            .unwrap();
        let entries = s.load_entries(id).unwrap();
        assert_eq!(entries.len(), 1, "unknown kind skipped, rest unaffected");
    }

    // ---- 分页 ----

    /// 尾巴分页：`load_tail` 只读最后 n 块，`load_before` 接着往前翻，
    /// 逐页拼起来必须与整读逐字一致。
    #[test]
    fn tail_paging_reassembles_the_transcript() {
        let mut s = mem_store();
        let id = s.create_session("t", "/tmp").unwrap();
        let es: Vec<Entry> = (0..10)
            .map(|i| user(&format!("m{i}")))
            .collect();
        s.append(id, &es).unwrap();
        let ids: Vec<i64> = s.stored_blocks(id).unwrap().iter().map(|b| b.id).collect();

        let (tail, more) = s.load_tail(id, 4).unwrap();
        assert_eq!(block_texts(&tail), vec!["m6", "m7", "m8", "m9"]);
        assert_eq!(tail[0].id, ids[6], "块自己带着 id：前端据此点名翻页");
        assert!(more, "上面还有 6 块");

        let (prev, more2) = s.load_before(id, tail[0].id, 4).unwrap();
        assert_eq!(block_texts(&prev), vec!["m2", "m3", "m4", "m5"]);
        assert_eq!(prev[0].id, ids[2]);
        assert!(more2);

        let (first, more3) = s.load_before(id, prev[0].id, 4).unwrap();
        assert_eq!(block_texts(&first), vec!["m0", "m1"]);
        assert!(!more3, "读到第一块了就不该说还有");

        let mut joined = first;
        joined.extend(prev);
        joined.extend(tail);
        assert_eq!(block_texts(&joined), page_texts(&es));
    }

    /// 往回滚：`load_after` 是 `load_before` 的镜像，拼起来同样逐字一致 ——
    /// 前端丢掉窗口另一头之后靠它把中间那段补回来。
    #[test]
    fn newer_paging_mirrors_the_older_one() {
        let mut s = mem_store();
        let id = s.create_session("t", "/tmp").unwrap();
        let es: Vec<Entry> = (0..10).map(|i| user(&format!("m{i}"))).collect();
        s.append(id, &es).unwrap();
        let ids: Vec<i64> = s.stored_blocks(id).unwrap().iter().map(|b| b.id).collect();

        // 从头往后读：0..3，再从 3 往后读 3..7，最后 7..10。
        let a = s.load_after(id, 0, 3).unwrap();
        assert_eq!(block_texts(&a), vec!["m0", "m1", "m2"]);
        let b = s.load_after(id, a.last().unwrap().id, 4).unwrap();
        assert_eq!(block_texts(&b), vec!["m3", "m4", "m5", "m6"]);
        let c = s.load_after(id, b.last().unwrap().id, 4).unwrap();
        assert_eq!(block_texts(&c), vec!["m7", "m8", "m9"]);
        assert_eq!(c.last().unwrap().id, ids[9]);
        assert!(s.load_after(id, ids[9], 4).unwrap().is_empty(), "到尾巴了");

        let joined: Vec<Block> = a.into_iter().chain(b).chain(c).collect();
        assert_eq!(block_texts(&joined), page_texts(&es));
    }

    /// 往回滚跨分叉点：父前缀的尾巴不能漏进来，子段接着往前走。
    #[test]
    fn newer_paging_crosses_the_fork_point() {
        let mut s = mem_store();
        let a = s.create_session("t", "/tmp").unwrap();
        s.append(a, &[user("a1"), user("a2"), user("a3")]).unwrap();
        let ids: Vec<i64> = s.stored_blocks(a).unwrap().iter().map(|b| b.id).collect();

        let b = s.fork_session(a, ids[1]).unwrap();
        s.append(b, &[user("b1")]).unwrap();

        // 站在父前缀的第一块上往后读：a2 该来（它在分叉点以内），a3 不该
        // （分叉在 a2 之后，父的后半截不属于这条支）。
        let after = s.load_after(b, ids[0], 8).unwrap();
        assert_eq!(block_texts(&after), vec!["a2", "b1"]);
    }

    /// 游标是块不是条目：一页永远不会把一次工具往返劈成两半，否则前端拿到的
    /// 第一块会是一条没有结果的请求。
    #[test]
    fn a_page_never_splits_a_tool_exchange() {
        let mut s = mem_store();
        let id = s.create_session("t", "/tmp").unwrap();
        s.append(
            id,
            &[req("c1"), res("c1"), req("c2"), res("c2"), req("c3"), res("c3")],
        )
        .unwrap();

        let (tail, more) = s.load_tail(id, 1).unwrap();
        assert_eq!(tail.len(), 1, "一块，里面是请求 + 结果");
        assert_eq!(tail[0].entries.len(), 2, "整块：请求 + 结果");
        assert!(matches!(tail[0].entries[0], Entry::ToolRequest { .. }));
        assert!(matches!(tail[0].entries[1], Entry::ToolResult { .. }));
        assert!(more);

        let (prev, more) = s.load_before(id, tail[0].id, 1).unwrap();
        assert_eq!(prev.len(), 1);
        assert!(matches!(prev[0].entries[0], Entry::ToolRequest { .. }));
        assert!(more);
    }

    // ---- 分叉 ----

    /// 分叉读的是"父前缀 + 自己的块"，父的废弃尾巴永远不进来。
    #[test]
    fn a_fork_reads_its_parents_prefix() {
        let mut s = mem_store();
        let a = s.create_session("t", "/tmp").unwrap();
        s.append(a, &[user("a1"), user("a2"), user("a3")]).unwrap();
        let ids: Vec<i64> = s.stored_blocks(a).unwrap().iter().map(|b| b.id).collect();

        let b = s.fork_session(a, ids[1]).unwrap();
        s.append(b, &[user("b1")]).unwrap();

        assert_eq!(texts_of(&s, b), vec!["a1", "a2", "b1"]);
        assert_eq!(texts_of(&s, a), vec!["a1", "a2", "a3"], "父不受影响");
    }

    /// 翻页要跨过分叉点：先给自己的块，再往回取父的前缀。
    #[test]
    fn a_fork_pages_across_the_fork_point() {
        let mut s = mem_store();
        let a = s.create_session("t", "/tmp").unwrap();
        s.append(a, &[user("a1"), user("a2"), user("a3")]).unwrap();
        let ids: Vec<i64> = s.stored_blocks(a).unwrap().iter().map(|b| b.id).collect();

        let b = s.fork_session(a, ids[1]).unwrap();
        s.append(b, &[user("b1")]).unwrap();

        let (tail, more) = s.load_tail(b, 2).unwrap();
        assert_eq!(block_texts(&tail), vec!["a2", "b1"]);
        assert!(more, "父前缀里还有 a1");
        let (older, more) = s.load_before(b, tail[0].id, 2).unwrap();
        assert_eq!(block_texts(&older), vec!["a1"]);
        assert!(!more);
    }

    /// 名字标记沿分支继承：分叉继承父当时的名字，自己命名后互不影响。
    #[test]
    fn a_fork_name_marker_is_inherited() {
        let mut s = mem_store();
        let a = s.create_session("t", "/tmp").unwrap();
        s.append(a, &[user("a1"), Entry::Name { name: "父名".into() }])
            .unwrap();
        let tip = s.tip(a).unwrap().unwrap();

        let b = s.fork_session(a, tip).unwrap();
        assert_eq!(s.effective_name(b).unwrap().as_deref(), Some("父名"));
        s.append(b, &[Entry::Name { name: "子名".into() }])
            .unwrap();
        assert_eq!(s.effective_name(b).unwrap().as_deref(), Some("子名"));
        assert_eq!(s.effective_name(a).unwrap().as_deref(), Some("父名"));
    }

    // ---- 删除 = 截断 ----

    /// 删父时，父被截到"活着的分支还需要的最后一块"，行留成墓碑——子分支
    /// 还能沿 `parent_id` 读到前缀。
    #[test]
    fn deleting_a_parent_truncates_to_what_its_forks_need() {
        let mut s = mem_store();
        let a = s.create_session("t", "/tmp").unwrap();
        s.append(a, &[user("a1"), user("a2"), user("a3")]).unwrap();
        let ids: Vec<i64> = s.stored_blocks(a).unwrap().iter().map(|b| b.id).collect();
        let b = s.fork_session(a, ids[1]).unwrap();

        assert!(s.delete_session(a).unwrap());
        assert_eq!(s.stored_blocks(a).unwrap().len(), 2, "a3 没人要了");
        assert_eq!(s.tip(a).unwrap(), Some(ids[1]));
        assert!(s.session(a).is_ok(), "墓碑行还在（子分支要沿链走）");
        assert!(
            s.list_sessions().unwrap().iter().all(|m| m.id != a),
            "墓碑不进列表"
        );
        assert_eq!(texts_of(&s, b), vec!["a1", "a2"], "前缀照旧可读");
    }

    /// 没人读的分支整棵消失：块、行、cwd 历史一起走。
    #[test]
    fn deleting_a_leaf_forgets_it_entirely() {
        let mut s = mem_store();
        let a = s.create_session("t", "/tmp").unwrap();
        s.append(a, &[user("a1")]).unwrap();
        assert!(s.delete_session(a).unwrap());
        assert!(s.list_sessions().unwrap().is_empty());
        assert!(s.session_uuid(a).unwrap().is_none());
        assert!(s.stored_blocks(a).unwrap().is_empty());
        assert!(!s.delete_session(a).unwrap(), "再删一次是个空操作");
    }

    /// 中间分支被删时不能把它孙子的前缀带走：它自己也变成"持有者"。
    #[test]
    fn a_grandchild_keeps_a_middle_branch_alive() {
        let mut s = mem_store();
        let a = s.create_session("t", "/tmp").unwrap();
        s.append(a, &[user("a1"), user("a2"), user("a3")]).unwrap();
        let ids: Vec<i64> = s.stored_blocks(a).unwrap().iter().map(|b| b.id).collect();

        let b = s.fork_session(a, ids[2]).unwrap();
        s.append(b, &[user("b1")]).unwrap();
        let b_tip = s.tip(b).unwrap().unwrap();
        let c = s.fork_session(b, b_tip).unwrap();

        s.delete_session(b).unwrap();
        assert_eq!(texts_of(&s, c), vec!["a1", "a2", "a3", "b1"]);

        s.delete_session(a).unwrap();
        assert_eq!(
            texts_of(&s, c),
            vec!["a1", "a2", "a3", "b1"],
            "删父之后孙子的前缀依然完整"
        );
    }

    // ---- 其余 ----

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
    fn the_picker_rows_carry_a_preview_and_a_size() {
        let mut s = mem_store();
        let a = s.create_session("2026-09-22 14:30:05", "/proj/a").unwrap();
        let b = s.create_session("2026-09-22 15:30:05", "/proj/b").unwrap();
        s.record_cwd(a, "/proj/shared").unwrap();
        s.append(
            a,
            &[
                user("改一下 todo\n第二行不该出现在预览里"),
                assistant("好"),
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
        let shared = s
            .list_session_rows(Some(std::path::Path::new("/proj/shared")))
            .unwrap();
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
        s.append(id, &[user("一")]).unwrap();
        s.record_cwd(id, "/tmp/clone-a").unwrap();
        s.append(id, &[user("二")]).unwrap();
        s.record_cwd(id, "/tmp/clone-b").unwrap();

        let h = s.cwd_history(id).unwrap();
        assert_eq!(h.len(), 3, "origin + two migrations");
        assert_eq!(h[0], (0, "/home/u/proj".to_string()));
        assert_eq!(h[1].1, "/tmp/clone-a");
        assert_eq!(h[2].1, "/tmp/clone-b");
        assert_eq!(h[1].0, 1, "换目录记在当次那一块上");
        // sessions.cwd must point at the final landing spot
        let meta = s.session(id).unwrap();
        assert_eq!(meta.cwd.as_deref(), Some("/tmp/clone-b"));
    }

    /// `context` 工具按**条目**位置标注窗口，所以轨迹里给的是条目下标：只有
    /// 存储知道一块是一条还是两条。工具往返算两条。
    #[test]
    fn the_cwd_trail_counts_entries_not_blocks() {
        let mut s = mem_store();
        let id = s.create_session("t", "/a").unwrap();
        s.append(id, &[user("一"), req("c1"), res("c1")]).unwrap();
        s.record_cwd(id, "/b").unwrap();
        s.append(id, &[user("二")]).unwrap();
        s.record_cwd(id, "/c").unwrap();

        assert_eq!(
            s.cwd_trail(id).unwrap(),
            vec![
                (0, "/a".to_string()),
                (3, "/b".to_string()),
                (4, "/c".to_string()),
            ]
        );
        assert_eq!(s.load_entries(id).unwrap().len(), 4, "下标对齐整读");
    }

    /// 分叉继承的轨迹位置照旧成立：前缀的条目在两边同样靠前。
    #[test]
    fn a_fork_inherits_the_cwd_trail_positions() {
        let mut s = mem_store();
        let a = s.create_session("t", "/a").unwrap();
        s.append(a, &[user("一")]).unwrap();
        s.record_cwd(a, "/b").unwrap();
        let tip = s.tip(a).unwrap().unwrap();
        let b = s.fork_session(a, tip).unwrap();
        s.append(b, &[user("二")]).unwrap();
        s.record_cwd(b, "/c").unwrap();

        assert_eq!(
            s.cwd_trail(b).unwrap(),
            vec![
                (0, "/a".to_string()),
                (1, "/b".to_string()),
                (2, "/c".to_string()),
            ]
        );
    }

    #[test]
    fn a_stored_round_replays_byte_identically_after_a_reopen() {
        // The database's whole contract: reopen the file and the conversation
        // comes back as the exact bytes the model saw.
        let (mut s, db) = file_store("bytes");
        let id = s.create_session("t", "/tmp").unwrap();
        let round = vec![
            user("问一下"),
            Entry::Reasoning {
                content: "先想\n再想".into(),
            },
            req("c1"),
            res("c1"),
            assistant("好了"),
        ];
        s.append(id, &round).unwrap();
        let tip = s.tip(id).unwrap();
        drop(s);

        let s = Store::open(&db).unwrap();
        let back = s.load_entries(id).unwrap();
        assert_eq!(back, round, "reopen must be byte-identical");
        assert_eq!(s.tip(id).unwrap(), tip);
        let ctx = crate::server::turn::entries_to_context("sys", &back);
        let wire = serde_json::to_string(&ctx.messages).unwrap();
        assert!(wire.contains("ok"), "{wire}");
        assert!(!wire.contains("先想"), "reasoning stays out of the protocol");
    }
}

#[cfg(test)]
mod name_tests {
    use super::*;
    use crate::server::entry::Entry;

    /// 名字标记：payload 往返 + 没有标记时退回 `sessions.name`。
    #[test]
    fn name_marker_round_trips_and_resolves() {
        let mut s = Store::open(std::path::Path::new(":memory:")).unwrap();
        let id = s.create_session("t", "/").unwrap();
        let (kind, payload) = Entry::Name { name: "x".into() }.to_payload();
        assert_eq!(kind, "name");
        assert_eq!(
            Entry::from_payload(kind, &payload).unwrap(),
            Entry::Name { name: "x".into() }
        );
        assert_eq!(s.effective_name(id).unwrap(), None, "还没命名");
        s.set_session_name(id, Some("列上的名")).unwrap();
        assert_eq!(s.effective_name(id).unwrap().as_deref(), Some("列上的名"));
    }
}
