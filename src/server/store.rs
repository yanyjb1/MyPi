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
use rusqlite::OptionalExtension;

use crate::server::entry::Entry;

/// One block as stored: 1 or 2 entries (a tool exchange is one block).
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub id: i64,
    pub ts: String,
    pub entries: Vec<Entry>,
}

/// 会话树上的一块：块本身 + 它**归属哪条会话**（也就是哪条 fork）。
///
/// 树状图要的就是这两样：块（画什么）和归属（缩进到哪一列）。归属是
/// `blocks.session_id`，不是"沿链找出来的"——一层 fork 一行 `sessions`，
/// 块自己记着它属于谁。
#[derive(Debug, Clone, PartialEq)]
pub struct TreeNode {
    pub block: Block,
    /// `blocks.session_id`：这块归哪条会话（fork）。
    pub session: i64,
}

/// 一条 fork 的元信息（画树要的那几列）。
#[derive(Debug, Clone, PartialEq)]
pub struct ForkInfo {
    pub id: i64,
    /// 亲会话。`None` = 这条链的根（主干）。
    pub parent: Option<i64>,
    /// 从亲会话的哪一块岔出来（含）。`None` = 根。
    pub fork_block: Option<i64>,
    /// 自己最后一块（`None` = 还没写过自己的块）。
    pub tip: Option<i64>,
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
    /// 这一行的**根**（沿 `parent_id` 爬到顶）。同一个根的行是同一个对话的
    /// 各条分支——选择器把它们合并成一条显示。
    pub root: i64,
    /// 这条分支自己的最后一块（`None` = 还没写过）。合并时用它比"谁最新"。
    pub tip: Option<i64>,
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
    /// 服务商名。`/usage` 按轮还原价格表时要先认出问的是哪一家。
    pub provider: String,
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
    /// The `compat:` block exactly as `models.yml` had it, or `None` when the
    /// entry carried none. Stored because detection is a pure function of
    /// `(base_url, model)` — recomputable — while a *hand-written override* is
    /// not: edit the config and a stored round would silently replay with
    /// different switches.
    pub compat_json: Option<String>,
}

/// 一轮的用量，按轮落盘（`turns` 的三个 token 列）。
///
/// 只存 token：钱是拿当前价格表算出来的，价格改了历史账单跟着改才对。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoundUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// `prompt_tokens` 里从缓存读的那部分。
    pub cached_tokens: u64,
}

/// One stored request header (a row of `turns`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoundRow {
    pub seq: i64,
    pub ts: String,
    pub model: String,
    /// 服务商名；老行为 `None`（那时候还没记）。
    pub provider: Option<String>,
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
    /// The `compat:` block as `models.yml` had it, serialized. `None` = the
    /// entry carried none (detection alone decided).
    pub compat_json: Option<String>,
    /// 这一轮的用量（`None` = 没拿到：半途死掉、网关不报）。
    pub usage: Option<RoundUsage>,
}

/// `/usage` 的一行：**一个模型**在同一会话里的累计用量与账单。
///
/// 按轮聚合而来（`turns` 按 `(provider, model)` 分组）。钱是服务端拿当前
/// 价格表算的——前端只负责画。
///
/// `Serialize` 是给 wire 用的（它就是面板上那一行）：面板数据不进转录、
/// 不落盘、不参与上下文，所以 wire 直接内嵌这个结构，不另立一份。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModelUsage {
    /// 展示名（配置里的 `name`，取不到就用 id）。
    pub name: String,
    /// 配置里的 id，认不出配置时前端还是能把行画出来。
    pub id: String,
    /// 所属服务商；`None` = 这一行没有服务商记录。
    pub provider: Option<String>,
    /// 问过几轮。
    pub rounds: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_tokens: u64,
    /// 四类 token 各自花的钱（当前价格表算的）。`None` = 认不出价格表。
    pub cost: Option<UsageCost>,
    /// 计价货币的展示符号（`¥` / `$`）。
    pub currency: Option<String>,
}

/// `/usage` 账单的四个格子。
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct UsageCost {
    /// 没命中缓存的输入。
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    /// 协议不报"写入缓存"，恒为 0；留着是为了账目四栏齐全。
    pub cache_write: f64,
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
    session: i64,
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

const ROUND_COLS: &str = "seq, ts, model, protocol, base_url, system, tools_json, max_tokens,
     stop_reason, first_block, last_block, provider, prompt_tokens, completion_tokens, cached_tokens,
     compat_json";

fn round_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RoundRow> {
    // 三个 token 列同生同死：网关要么整块报 usage，要么一个字都不报，所以
    // 拿 prompt 那列探有没有，缺的按 0 补——不引入"半个用量"这种状态。
    let prompt: Option<i64> = r.get(12)?;
    let completion: Option<i64> = r.get(13)?;
    let cached: Option<i64> = r.get(14)?;
    let usage = prompt.map(|p| RoundUsage {
        prompt_tokens: p.max(0) as u64,
        completion_tokens: completion.unwrap_or(0).max(0) as u64,
        cached_tokens: cached.unwrap_or(0).max(0) as u64,
    });
    Ok(RoundRow {
        seq: r.get(0)?,
        ts: r.get(1)?,
        model: r.get(2)?,
        provider: r.get(11)?,
        protocol: r.get(3)?,
        base_url: r.get(4)?,
        system: r.get(5)?,
        tools_json: r.get(6)?,
        max_tokens: r.get(7)?,
        stop_reason: r.get(8)?,
        first_block: r.get(9)?,
        last_block: r.get(10)?,
        compat_json: r.get(15)?,
        usage,
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
        const SCHEMA_VERSION: i64 = 4;
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
                -- 会话级模型：**服务商名** + 模型 id（裸的，不带服务商前缀）。
                -- NULL = 跟着配置的默认模型走。`/switch` 写它、resume 读它：这是
                -- 这是这个会话在用什么模型，和 `/model` 改的默认配置是两回事。
                -- 存名字不存 endpoint 和密钥——密钥不该落盘，配置改了也该跟着走。
                -- 服务商单独存：不同服务商会有同名模型，缺了它恢复时认不出谁是谁。
                model_provider TEXT,
                model         TEXT,
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
                -- 这一轮用的兼容开关（`models.yml` 里那个 `compat:` 块）。
                -- 存**用户写的**那份，不存探测结果：探测是 (baseUrl, model)
                -- 的纯函数，重放时重算即可；用户写的那份在配置改掉之后就
                -- 找不回来了，所以它是数据，得落盘。
                compat_json TEXT,
                stop_reason TEXT,
                first_block INTEGER,
                last_block  INTEGER,
                -- 这一轮问的是谁。「模型」在配置里只是**某个服务商名下**的一条
                -- 记录（同名模型可以有两家），裸 id 认不出该问谁问价；价格表
                -- 归配置管，`/usage` 要按轮还原当时的价格，就得先认出服务商。
                provider    TEXT,
                -- 这一轮的用量。**只存 token 不存钱**：token 是事实，钱是拿
                -- 当前价格表算出来的——价格改了，历史账单跟着改才对。
                -- NULL = 没拿到（半途死掉、网关不报 usage）。
                prompt_tokens     INTEGER,
                completion_tokens INTEGER,
                -- prompt_tokens 里从缓存读的那部分（计费比普通输入便宜）。
                cached_tokens     INTEGER,
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

    /// 这个会话在用什么模型：`(服务商名, 裸模型 id)`。`None` = 跟着配置默认走。
    ///
    /// 两半缺一不可——不同服务商会有同名模型，只记一个 id 的话，恢复时认不出
    /// 该去哪家问，报错也说不清是谁的问题。
    pub fn session_model(&self, session_id: i64) -> Result<Option<(String, String)>> {
        let v: Option<(Option<String>, Option<String>)> = self
            .conn
            .query_row(
                "SELECT model_provider, model FROM sessions WHERE id = ?1",
                [session_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(v.and_then(|(p, m)| Some((p?, m?))))
    }

    /// 记下这个会话切到了哪个模型（`None` = 回到跟着默认走）。
    pub fn set_session_model(
        &mut self,
        session_id: i64,
        model: Option<(&str, &str)>,
    ) -> Result<()> {
        let (provider, id) = match model {
            Some((p, m)) => (Some(p), Some(m)),
            None => (None, None),
        };
        self.conn.execute(
            "UPDATE sessions SET model_provider = ?1, model = ?2 WHERE id = ?3",
            rusqlite::params![provider, id, session_id],
        )?;
        Ok(())
    }

    /// Fork `parent_id` at `fork_block_id` (inclusive): the new session inherits
    /// every block up to and including it and owns none of its own yet.
    ///
    /// Nothing is copied. A fork is one row, so forking a 30 000-block
    /// conversation costs the same as forking a two-message one.
    pub fn fork_session(&mut self, parent_id: i64, fork_block_id: i64) -> Result<i64> {
        anyhow::ensure!(
            self.block_on_branch(parent_id, fork_block_id)?,
            "no block {fork_block_id} on session {parent_id}'s branch"
        );
        let cwd = self.session(parent_id)?.cwd.unwrap_or_default();
        let tx = self.conn.transaction()?;
        tx.execute(
            "INSERT INTO sessions (uuid, started_at, cwd, parent_id, fork_block_id,
                                   model_provider, model)
             SELECT ?1, ?2, ?3, ?4, ?5, model_provider, model FROM sessions WHERE id = ?4",
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

    /// **截断**会话到 `block_id`（含）——「回退」的剪枝做法。
    ///
    /// 与 [`Self::fork_session`] 相对：分叉是**留下**被抛下的内容（新开一行，
    /// 旧行原样），随时能回去；截断是**扔掉**它——自己名下更晚的块、那之后的
    /// 回合记录、cwd 历史、产物一起删掉，tip 和体积重算。同一个会话行内完成，
    /// 所以回退多少次链都不会变长。
    ///
    /// `block_id` 落在**继承来的前缀**里时（回退到分叉点之前），自己名下的块
    /// 一块不剩，会话就地**重新生根**：`parent_id` 指到那一块所属的会话上。
    /// 这样回退到哪，读出来的前缀就到哪。
    ///
    /// 不允许回退到自己**后代**的块上：那是把历史接到未来上，还会让
    /// `parent_id` 成环（`chain` 会转圈）。
    /// 返回**被整条砍掉的会话 id**（回退点之后岔出去的那些）——调用方要把
    /// 还开在内存里的句柄一并丢掉。
    pub fn prune_session(&mut self, session_id: i64, block_id: i64) -> Result<Vec<i64>> {
        let owner: Option<i64> = self
            .conn
            .query_row(
                "SELECT session_id FROM blocks WHERE block_id = ?1",
                [block_id],
                |r| r.get(0),
            )
            .optional()?;
        let owner = owner.ok_or_else(|| anyhow::anyhow!("block {block_id} does not exist"))?;
        if owner != session_id {
            anyhow::ensure!(
                !self.is_descendant(owner, session_id)?,
                "cannot rewind onto a descendant (block {block_id} belongs to session {owner})"
            );
        }
        // **整棵树一起砍**，不只看自己名下那几块：前几次回退岔出去的分支和
        // 这个会话同根，在用户眼里就是同一段历史的几种走法——只砍自己的，
        // resume 一看旧内容全还在（这正是"没看到剪枝效果"的来源）。
        let family = self.subtree(self.root_of(session_id)?)?;
        // 事务前读：哪些会话会被砍空——回退点之前一块自己的都没有，落点又落
        // 在被砍的那一段里；或者干脆挂在另一个被砍空的分支上（连坐）。
        // `family` 是 BFS 序，父一定排在孩子前面，所以一趟就能连坐到底。
        let mut doomed: std::collections::HashSet<i64> = std::collections::HashSet::new();
        for &id in &family {
            if id == session_id {
                continue;
            }
            let (parent, fork, _) = self.branch_row(id)?;
            let keep: i64 = self.conn.query_row(
                "SELECT COUNT(*) FROM blocks WHERE session_id = ?1 AND block_id <= ?2",
                rusqlite::params![id, block_id],
                |r| r.get(0),
            )?;
            if parent.is_some_and(|p| doomed.contains(&p))
                || (keep == 0 && fork.is_some_and(|f| f > block_id))
            {
                doomed.insert(id);
            }
        }
        let tx = self.conn.transaction()?;
        for &id in &family {
            tx.execute(
                "DELETE FROM blocks WHERE session_id = ?1 AND block_id > ?2",
                rusqlite::params![id, block_id],
            )?;
            // 还没随回合落盘的产物（block_id IS NULL）属于被截掉的那一段。
            tx.execute(
                "DELETE FROM artifacts WHERE session_id = ?1
                   AND (block_id IS NULL OR block_id > ?2)",
                rusqlite::params![id, block_id],
            )?;
            tx.execute(
                "DELETE FROM turns WHERE session_id = ?1 AND first_block > ?2",
                rusqlite::params![id, block_id],
            )?;
            // 换目录的记录跟着块走：块没了，那条记录的落点也就不存在了。
            tx.execute(
                "DELETE FROM cwd_history WHERE session_id = ?1 AND block_id > ?2",
                rusqlite::params![id, block_id],
            )?;
        }
        // 先给自己换好新的挂载点，再删被砍空的行——否则自己要指向的那一行
        // 可能正被删掉（外键会拦）。
        if owner != session_id {
            tx.execute(
                "UPDATE sessions SET parent_id = ?1, fork_block_id = ?2 WHERE id = ?3",
                rusqlite::params![owner, block_id, session_id],
            )?;
        }
        // 深的先走（`family` 是 BFS 序，反过来就是叶子先）。这些行整条不要
        // 了，关联表得**整个清空**——`cwd_history` 的会话原点行（block_id 0）
        // 和第一回合的 `turns` 都在回退点之前，只按 `> block_id` 删会留下
        // 悬空引用，外键当场拦下。
        let mut dropped: Vec<i64> = Vec::new();
        for &id in family.iter().rev() {
            if !doomed.contains(&id) {
                continue;
            }
            for table in ["blocks", "cwd_history", "artifacts", "turns"] {
                tx.execute(&format!("DELETE FROM {table} WHERE session_id = ?1"), [id])?;
            }
            tx.execute("DELETE FROM sessions WHERE id = ?1", [id])?;
            dropped.push(id);
        }
        tx.execute(
            "UPDATE sessions
                SET tip_block_id = (SELECT MAX(block_id) FROM blocks WHERE session_id = ?1),
                    bytes = (SELECT COALESCE(SUM(LENGTH(payload)
                                + COALESCE(LENGTH(payload2), 0)), 0)
                               FROM blocks WHERE session_id = ?1)
              WHERE id = ?1",
            [session_id],
        )?;
        tx.commit()?;
        Ok(dropped)
    }

    /// `candidate` 是不是 `root` 的后代（沿 `parent_id` 链能走到 `root`）。
    ///
    /// 带访问集合：真要出现环时返回 false，而不是转圈。
    fn is_descendant(&self, candidate: i64, root: i64) -> Result<bool> {
        let mut seen = std::collections::HashSet::new();
        let mut cur = Some(candidate);
        while let Some(id) = cur {
            if id == root {
                return Ok(true);
            }
            if !seen.insert(id) {
                return Ok(false);
            }
            cur = self.branch_row(id)?.0;
        }
        Ok(false)
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
        usage: Option<RoundUsage>,
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
                                tools_json, max_tokens, stop_reason, first_block, last_block,
                                provider, prompt_tokens, completion_tokens, cached_tokens,
                                compat_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
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
                last_block,
                meta.provider,
                usage.map(|u| u.prompt_tokens as i64),
                usage.map(|u| u.completion_tokens as i64),
                usage.map(|u| u.cached_tokens as i64),
                meta.compat_json,
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    // Every stored request header of a session, oldest first.
    pub fn rounds(&self, session_id: i64) -> Result<Vec<RoundRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {ROUND_COLS} FROM turns WHERE session_id = ?1 ORDER BY seq"
        ))?;
        let rows = stmt.query_map([session_id], round_from_row)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    /// `/usage` 的数据源：本会话的用量**按模型**聚合成几行。
    ///
    /// 以轮为单位（`turns`），因为模型是**会话级**的东西、一轮之内不变——
    /// 分组键因此是 `(provider, model)` 而不是逐块，顺带也就是"这个会话换过
    /// 几个模型、各花了多少"。金额不在这里算：价格表归配置管（会话层去解析），
    /// 存储只认 token 这个事实。
    ///
    /// 没拿到用量的轮（`prompt_tokens IS NULL`）计入 `rounds` 但 token 记 0：
    /// "问过一次、账单没记上"本身是要说出来的信息，藏掉它就等于假装没花过钱。
    ///
    /// 分叉不复制 `turns`（见 [`Self::fork_session`]），所以这里天然只算
    /// **本分支**的账。
    pub fn usage_by_model(&self, session_id: i64) -> Result<Vec<ModelUsage>> {
        let mut stmt = self.conn.prepare(
            "SELECT provider, model,
                    COUNT(*),
                    COALESCE(SUM(prompt_tokens), 0),
                    COALESCE(SUM(completion_tokens), 0),
                    COALESCE(SUM(cached_tokens), 0)
               FROM turns WHERE session_id = ?1
              GROUP BY provider, model
              ORDER BY MIN(seq)",
        )?;
        let rows = stmt.query_map([session_id], |r| {
            let provider: Option<String> = r.get(0)?;
            let id: String = r.get(1)?;
            Ok(ModelUsage {
                name: id.clone(),
                id,
                provider,
                rounds: r.get::<_, i64>(2)?.max(0) as u64,
                prompt_tokens: r.get::<_, i64>(3)?.max(0) as u64,
                completion_tokens: r.get::<_, i64>(4)?.max(0) as u64,
                cached_tokens: r.get::<_, i64>(5)?.max(0) as u64,
                cost: None,
                currency: None,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    // One stored request header, or None when the turn is not stored.
    pub fn round(&self, session_id: i64, seq: i64) -> Result<Option<RoundRow>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {ROUND_COLS} FROM turns WHERE session_id = ?1 AND seq = ?2"
        ))?;
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

    /// 一整棵树里**所有**块，按时序（`block_id`）排，每块带着它的归属
    /// 会话（fork）；外加树里每条 fork 的元信息。
    ///
    /// 「一棵树」= 从 `parent_id IS NULL` 的**原始主干**起，往下收全部后代。
    /// `session_id` 只用来回答"我在哪一块"（`current`），不影响收哪些块——
    /// 回到历史节点时，看得见的应当是全部分支，不是自己那一条。
    ///
    /// 块按 `block_id` 排就自然成了时序：`block_id` 是**全局自增主键**，
    /// 跨会话唯一，所以"谁先谁后"不用另算时间戳，也**不可能重复**——
    /// 一个块在 `blocks` 里就一行，树的公共前缀画一次就是一次。
    pub fn lineage(&self, session_id: i64) -> Result<(Vec<TreeNode>, Vec<ForkInfo>, i64)> {
        // 爬到根（`parent_id IS NULL` = 原始主干）。一个库可以有多棵树，
        // 只收 `session_id` 所在的那一棵。
        let root = self.root_of(session_id)?;

        // 从根往下收整棵树，每层记下它继承的前缀上界（`chain` 的第一段）
        // 与自己的 `tip`——收块的范围就是 (前缀上界, tip]。
        let mut all: Vec<ForkInfo> = Vec::new();
        let mut frontier = vec![root];
        while let Some(sid) = frontier.pop() {
            let (parent, fork, tip) = self.branch_row(sid)?;
            all.push(ForkInfo {
                id: sid,
                parent,
                fork_block: fork,
                tip,
            });
            for child in self.children_of(sid)? {
                frontier.push(child);
            }
        }
        all.sort_by_key(|f| f.id);

        // 每棵树只查一次全量、按 `block_id` 天然去重。逐 session 分开查再拼
        // 会有两个坑：前缀块查不到（它存在祖先的 `session_id` 下）、以及
        // 排序键跨会话不可比。这里的 `session_id` 只用来标记归属。
        let ids: Vec<rusqlite::types::Value> = all.iter().map(|f| f.id.into()).collect();
        let placeholders = (1..=ids.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut nodes = Vec::new();
        for raw in self.read_raw(
            &format!(
                "SELECT {BLOCK_COLS}, session_id FROM blocks
                  WHERE session_id IN ({placeholders}) ORDER BY block_id"
            ),
            ids,
        )? {
            let id = raw.id;
            let session = raw.session;
            let ts = raw.ts.clone();
            let (entries, _ok) = raw.into_entries();
            nodes.push(TreeNode {
                block: Block { id, ts, entries },
                session,
            });
        }

        // "我在哪"：当前这条线自己的最后一块。树里可能同时有好几条平行分支，
        // `session_id` 的 `tip` 才是"我停在的地方"。
        let current = self
            .tip(session_id)?
            .or_else(|| nodes.last().map(|n| n.block.id))
            .unwrap_or(0);
        Ok((nodes, all, current))
    }

    /// 沿 `parent_id` 爬到顶，返回原始主干（`parent_id IS NULL`）的 id。
    fn root_of(&self, session_id: i64) -> Result<i64> {
        let mut cur = session_id;
        loop {
            let (parent, _, _) = self.branch_row(cur)?;
            match parent {
                Some(p) => cur = p,
                None => return Ok(cur),
            }
        }
    }

    /// 直接以 `parent_id` 挂在这条会话下面的子会话。
    fn children_of(&self, id: i64) -> Result<Vec<i64>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT id FROM sessions WHERE parent_id = ?1 ORDER BY id")?;
        let rows = stmt.query_map([id], |r| r.get::<_, i64>(0))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }


    ///
    /// 冷启动第一帧只要一屏：32k 块脏库上整读要 261 ms（读 81 + 解析 81 +
    /// 路径走查 ~100），而真正为第一帧服务的是最后那几十块。这条只读尾巴，
    /// 而且**从叶子段往回走**——每段一次索引区间扫，分叉深度是几就几跳。
    pub fn load_tail(&self, session_id: i64, limit: usize) -> Result<(Vec<Block>, bool)> {
        self.page(session_id, None, limit)
    }

    /// 这条会话**最后一条回复**记下的上下文大小（`None` = 一条都没有）。
    ///
    /// 恢复历史时用它当用量起点：内存里的实测值是新的（进程刚起来），而库里
    /// 那段对话早就不止开场白那么长了——拿开场白的体重顶上去，表会指着一个
    /// 明显偏小的数。
    ///
    /// 按轮倒着取第一条有量的记录（`turns.seq` 是会话内的时序键），不解析
    /// 块的 JSON。
    pub fn last_prompt_tokens(&self, session_id: i64) -> Result<Option<u64>> {
        let n: Option<i64> = self
            .conn
            .query_row(
                "SELECT prompt_tokens FROM turns
                  WHERE session_id = ?1 AND prompt_tokens IS NOT NULL
                  ORDER BY seq DESC LIMIT 1",
                [session_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(n.filter(|v| *v > 0).map(|v| v as u64))
    }

    /// 同一份开场白（系统提示词 + 工具手册 + 模型）在**历史回合**里实测到的
    /// 最小上下文占用。`None` = 一次都没实测过。
    ///
    /// 最小值就是开场白的体重：`prompt_tokens` 随对话变长只涨不跌，所以同一份
    /// 开场白在所有记录里的下界就是"还没说话时"的占用。新会话在第一轮请求
    /// 回来之前拿它当预估值，比按字符瞎折准得多。
    ///
    /// 匹配用**逐字相等**（`turns` 表存的就是请求里那一份原文），所以换一个
    /// 字符都会落空——那正是要的：换过 profile 的会话不该借旧数据。
    ///
    /// 用量按轮存在 `turns` 上，所以这一问是一行聚合，不碰 `blocks` 的 JSON
    /// 载荷，也不 JOIN。
    pub fn min_prompt_tokens(
        &self,
        system: &str,
        tools_json: &str,
        model: &str,
    ) -> Result<Option<u64>> {
        let n: Option<i64> = self.conn.query_row(
            "SELECT MIN(prompt_tokens) FROM turns
              WHERE system = ?1 AND tools_json = ?2 AND model = ?3
                AND prompt_tokens IS NOT NULL",
            rusqlite::params![system, tools_json, model],
            |r| r.get(0),
        )?;
        Ok(n.filter(|v| *v > 0).map(|v| v as u64))
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
            .with_context(|| format!("session {id} does not exist"))
    }

    fn read_raw(&self, sql: &str, params: Vec<rusqlite::types::Value>) -> Result<Vec<RawBlock>> {
        let mut stmt = self.conn.prepare_cached(sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params), |r| {
            Ok(RawBlock {
                id: r.get(0)?,
                session: r.get(6).unwrap_or(0),
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
        // 一条 SQL 里把"根"算出来（递归 CTE 沿 parent_id 爬到顶），再在 Rust
        // 侧按根分组。选择器一行 = 一个对话 = 一个根的全部分支。
        //
        // 每行的预览/体积/时间取自**该根里最新的那条分支**（tip_block_id 最大，
        // 没有 tip 的算最旧）——用户上次停在哪儿，列表就该显示哪儿。
        let sql = "WITH RECURSIVE up(id, root) AS (
                        SELECT id, id FROM sessions WHERE parent_id IS NULL
                        UNION ALL
                        SELECT s.id, up.root FROM sessions s
                          JOIN up ON s.parent_id = up.id
                    )
                    SELECT s.id, s.name, s.started_at, s.cwd,
                           (SELECT b.payload FROM blocks b
                             WHERE b.session_id = s.id AND b.kind = 'user'
                             ORDER BY b.block_id LIMIT 1),
                           s.bytes, up.root, s.tip_block_id
                      FROM sessions s JOIN up ON up.id = s.id
                     WHERE s.deleted_at IS NULL
                       AND (?1 IS NULL
                            OR s.id IN (SELECT DISTINCT session_id
                                          FROM cwd_history WHERE cwd = ?1))
                     ORDER BY s.id DESC";
        let mut stmt = self.conn.prepare(sql)?;
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
                root: r.get(6)?,
                tip: r.get(7)?,
            })
        })?;
        let rows = rows.collect::<std::result::Result<Vec<_>, _>>()?;

        // 按根分组，每条分支留**最新**的那个（tip 大的赢；都没 tip 就留 id 大的，
        // 也就是后建的）。分支自己的体积加总——"这个对话一共多大"。
        let mut order: Vec<i64> = Vec::new();
        let mut best: std::collections::HashMap<i64, SessionRow> = std::collections::HashMap::new();
        for row in rows {
            let key = row.root;
            match best.get_mut(&key) {
                None => {
                    order.push(key);
                    best.insert(key, row);
                }
                Some(prev) => {
                    prev.bytes += row.bytes;
                    let newer = (row.tip.unwrap_or(-1), row.meta.id)
                        > (prev.tip.unwrap_or(-1), prev.meta.id);
                    if newer {
                        let summed = prev.bytes;
                        let mut winner = row;
                        winner.bytes = summed;
                        *prev = winner;
                    }
                }
            }
        }
        // 保持"新的在前"：按合并后那一行的 id 降序（id 全局自增，等价于建行时序）。
        let mut out: Vec<SessionRow> = best.into_values().collect();
        out.sort_by_key(|r| std::cmp::Reverse(r.meta.id));
        let _ = order;
        Ok(out)
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
// Pure arithmetic, no `date` fork and no date crate: this runs on every
// session creation and every picker sort, and a fork+exec per call is a
// measurable cost in the daemon's hot path. Local time comes from the
// `TZ`-aware offset the C library reports (`localtime_r` is not reachable
// without a dep, so the offset is read once per call from the same source
// `date` uses: `tm_gmtoff` via `SystemTime` + the process's TZ offset).
pub fn now_stamp() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    stamp_from_unix(secs + local_offset_secs())
}

/// `YYYY-MM-DD HH:MM:SS` for a Unix timestamp **already shifted into local
/// time**. Split out so the civil-date arithmetic is testable without a
/// clock or a timezone.
fn stamp_from_unix(local: i64) -> String {
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 → (y, m, d).
/// Floored arithmetic, so it is correct for negative (pre-1970) inputs too.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097); // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Seconds to add to UTC to get the process's local time.
///
/// Read from the `TZ` environment when it names a fixed offset (`UTC+8`,
/// `GMT-5`); otherwise 0. A named zone (`Asia/Shanghai`) needs a tz database
/// this crate does not link, and the stamps are display-only — a wrong one
/// costs a skewed picker label, never a logic comparison (see `age_seconds`,
/// which parses the real delta back out and reports `None` on anything it
/// cannot read).
fn local_offset_secs() -> i64 {
    let Ok(tz) = std::env::var("TZ") else {
        return 0;
    };
    let tz = tz.trim();
    let rest = tz
        .strip_prefix("UTC")
        .or_else(|| tz.strip_prefix("GMT"))
        .unwrap_or(tz);
    let Some((sign, digits)) = rest.split_at_checked(1) else {
        return 0;
    };
    let mult = match sign {
        "+" => -1,
        "-" => 1,
        _ => return 0,
    };
    // POSIX sign convention: UTC+8 means 8 hours **west** of UTC.
    let (h, m) = match digits.split_once(':') {
        Some((h, m)) => (h, m),
        None => (digits, "0"),
    };
    let (Ok(h), Ok(m)) = (h.parse::<i64>(), m.parse::<i64>()) else {
        return 0;
    };
    mult * (h * 3600 + m * 60)
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
/// `None` when the stamp is malformed — the picker then shows the raw stamp
/// instead of a wrong age. The parse is the inverse of [`stamp_from_unix`]:
/// days-from-civil plus the time of day, shifted back out of local time.
pub fn age_seconds(stamp: &str) -> Option<u64> {
    let local = parse_stamp(stamp)?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs() as i64;
    let now_local = now + local_offset_secs();
    Some(now_local.saturating_sub(local).max(0) as u64)
}

/// `YYYY-MM-DD HH:MM:SS` (local) → local unix seconds.
///
/// Hand-parsed rather than passed to `date`: the shape is fixed and our own,
/// and a fork per row is exactly what `now_stamp` just stopped paying.
fn parse_stamp(stamp: &str) -> Option<i64> {
    let b = stamp.as_bytes();
    // "YYYY-MM-DD HH:MM:SS" = 19 bytes; a trailing fraction is ignored.
    if b.len() < 19 {
        return None;
    }
    let num = |a: usize, z: usize| -> Option<i64> {
        let s = std::str::from_utf8(&b[a..z]).ok()?;
        s.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| s.parse().ok())?
    };
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (h, mi, s) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if b[4] != b'-' || b[7] != b'-' || b[10] != b' ' || b[13] != b':' || b[16] != b':' {
        return None;
    }
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    let days = days_from_civil(y, mo as u32, d as u32);
    Some(days * 86_400 + h * 3600 + mi * 60 + s)
}

/// Inverse of [`civil_from_days`]: (y, m, d) → days since 1970-01-01.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400; // [0, 399]
    let mp = if m > 2 { m - 3 } else { m + 9 } as i64; // [0, 11]
    let doy = (153 * mp + 2) / 5 + d as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
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

    /// 会话级模型：写进去、读回来；`None` = 跟着配置的默认走。
    #[test]
    fn the_session_model_round_trips() {
        let mut s = mem_store();
        let a = s.create_session("t0", "/tmp").unwrap();
        assert_eq!(s.session_model(a).unwrap(), None, "新会话跟着默认走");
        s.set_session_model(a, Some(("vendor-a", "model-b"))).unwrap();
        assert_eq!(
            s.session_model(a).unwrap(),
            Some(("vendor-a".to_string(), "model-b".to_string()))
        );
        s.set_session_model(a, None).unwrap();
        assert_eq!(s.session_model(a).unwrap(), None, "切回默认就是 NULL");
    }

    /// 同一份开场白在历史里的**最小**实测占用会被借出来：`prompt_tokens` 随
    /// 对话只涨不跌，所以历史里的下界就是"还没说话时"的体重——新会话在第一
    /// 轮响应回来之前，用量表拿它当预估值。
    #[test]
    fn the_smallest_recorded_prompt_is_borrowed() {
        let mut s = mem_store();
        let a = s.create_session("t0", "/tmp").unwrap();
        let meta = |model: &str, system: &str| RoundMeta {
            model: model.into(),
            provider: "test".into(),
            protocol: "openai-chat-completions".into(),
            base_url: "http://x/v1".into(),
            system: system.into(),
            tools_json: "[]".into(),
            max_tokens: 4096,
            compat_json: None,
        };
        let used = |n: u64| Entry::Assistant {
            content: "答".into(),
            usage: Some(crate::server::entry::UsageSummary {
                total_tokens: n + 5,
                prompt_tokens: n,
                cached_tokens: 0,
                completion_tokens: 5,
                reasoning_tokens: 0,
            }),
        };
        s.append_round(
            a,
            &[user("一"), used(900)],
            &meta("m", "SYS"),
            None,
            Some(RoundUsage {
                prompt_tokens: 900,
                completion_tokens: 5,
                cached_tokens: 0,
            }),
        )
            .unwrap();
        s.append_round(
            a,
            &[user("二"), used(1400)],
            &meta("m", "SYS"),
            None,
            Some(RoundUsage {
                prompt_tokens: 1400,
                completion_tokens: 5,
                cached_tokens: 0,
            }),
        )
            .unwrap();

        assert_eq!(
            s.min_prompt_tokens("SYS", "[]", "m").unwrap(),
            Some(900),
            "取历史最小值，不是最近一次"
        );
        // 开场白、工具手册、模型任意一处不同都落空：换过 profile 的会话不该
        // 借旧数据（字数差一个也不行，比的是原文）。
        assert_eq!(s.min_prompt_tokens("SYS!", "[]", "m").unwrap(), None);
        assert_eq!(s.min_prompt_tokens("SYS", "[{}]", "m").unwrap(), None);
        assert_eq!(s.min_prompt_tokens("SYS", "[]", "m2").unwrap(), None);
    }

    /// 恢复历史时的起点：库里**最后**一条回复记录的上下文大小（不是最小值
    /// ——那段对话早就长过开场白了）。
    #[test]
    fn the_last_recorded_prompt_serves_a_resumed_session() {
        let mut s = mem_store();
        let a = s.create_session("t0", "/tmp").unwrap();
        let meta = RoundMeta {
            model: "m".into(),
            provider: "test".into(),
            protocol: "openai-chat-completions".into(),
            base_url: "http://x/v1".into(),
            system: "SYS".into(),
            tools_json: "[]".into(),
            max_tokens: 4096,
            compat_json: None,
        };
        let used = |n: u64| Entry::Assistant {
            content: "答".into(),
            usage: Some(crate::server::entry::UsageSummary {
                total_tokens: n + 5,
                prompt_tokens: n,
                cached_tokens: 0,
                completion_tokens: 5,
                reasoning_tokens: 0,
            }),
        };
        let usage = |n: u64| {
            Some(RoundUsage {
                prompt_tokens: n,
                completion_tokens: 5,
                cached_tokens: 0,
            })
        };
        s.append_round(a, &[user("一"), used(900)], &meta, None, usage(900))
            .unwrap();
        s.append_round(a, &[user("二"), used(1400)], &meta, None, usage(1400))
            .unwrap();
        assert_eq!(s.last_prompt_tokens(a).unwrap(), Some(1400));

        let empty = s.create_session("t0", "/tmp").unwrap();
        assert_eq!(s.last_prompt_tokens(empty).unwrap(), None, "没跑过就没有");
    }

    /// `/usage` 的数据源：用量按 `(服务商, 模型)` 分组累加。
    ///
    /// 两件事要同时成立——同一个模型跨轮累加，同名模型跨服务商**不**串。
    /// 后者是分组键要带服务商的原因：`Ali-dashscope/Qwen3.8` 和别家的同名
    /// 条目是两笔账。
    #[test]
    fn usage_is_summed_per_model_and_provider() {
        let mut s = mem_store();
        let a = s.create_session("t0", "/tmp").unwrap();
        let meta = |model: &str, provider: &str| RoundMeta {
            model: model.into(),
            provider: provider.into(),
            protocol: "openai-chat-completions".into(),
            base_url: "http://x/v1".into(),
            system: "SYS".into(),
            tools_json: "[]".into(),
            max_tokens: 4096,
            compat_json: None,
        };
        let u = |p: u64, c: u64, r: u64| RoundUsage {
            prompt_tokens: p,
            completion_tokens: c,
            cached_tokens: r,
        };
        s.append_round(a, &[user("一")], &meta("m1", "p1"), None, Some(u(10, 2, 0)))
            .unwrap();
        s.append_round(a, &[user("二")], &meta("m1", "p1"), None, Some(u(20, 4, 8)))
            .unwrap();
        s.append_round(a, &[user("三")], &meta("m2", "p1"), None, Some(u(5, 1, 0)))
            .unwrap();
        // 半途死掉的那一轮：用量没拿到，但它确实问过——不记 token，也别
        // 装作没有过这一轮。
        s.append_round(a, &[user("四")], &meta("m2", "p1"), None, None)
            .unwrap();

        let g = s.usage_by_model(a).unwrap();
        assert_eq!(g.len(), 2, "两个模型两行：{g:?}");
        assert_eq!((g[0].provider.as_deref(), g[0].id.as_str()), (Some("p1"), "m1"));
        assert_eq!(g[0].rounds, 2);
        assert_eq!(g[0].prompt_tokens, 30, "跨轮累加");
        assert_eq!(g[0].completion_tokens, 6);
        assert_eq!(g[0].cached_tokens, 8);
        assert_eq!(g[1].rounds, 2, "没拿到用量的那轮也算问过一次");
        assert_eq!(g[1].prompt_tokens, 5);

        // 同名模型挂两家：分组键是 (provider, model)，不是 model。
        let b = s.create_session("t0", "/tmp").unwrap();
        s.append_round(b, &[user("一")], &meta("m1", "p1"), None, Some(u(1, 1, 0)))
            .unwrap();
        s.append_round(b, &[user("二")], &meta("m1", "p2"), None, Some(u(2, 2, 0)))
            .unwrap();
        let g = s.usage_by_model(b).unwrap();
        assert_eq!(g.len(), 2, "不同服务商的同名模型不许合并：{g:?}");

        // 账本按会话分：另一个会话的轮不能混进来。
        assert_eq!(s.usage_by_model(a).unwrap()[0].prompt_tokens, 30);
    }

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

    /// 分叉之后，`lineage` 必须把**两条分支的块**都列出来（按时序），
    /// 且每块带着自己的归属会话——这是节点树算缩进的唯一依据。
    #[test]
    fn lineage_lists_both_branches_with_their_own_session() {
        let mut s = mem_store();
        let a = s.create_session("t", "/tmp").unwrap();
        // 主干：A1 A2
        s.append(a, &[user("A1")]).unwrap();
        let tip_a = s.tip(a).unwrap().unwrap();
        s.append(a, &[user("A2")]).unwrap();
        let fork_point = s.tip(a).unwrap().unwrap();

        // 从 A2 岔出去，新会话写 B1 B2。
        let b = s.fork_session(a, fork_point).unwrap();
        assert_ne!(a, b);
        s.append(b, &[user("B1"), user("B2")]).unwrap();

        let (nodes, forks, _current) = s.lineage(a).unwrap();

        // 两条会话都在 fork 列表里，b 认 a 做亲。
        let fa = forks.iter().find(|f| f.id == a).expect("主干要在树里");
        let fb = forks.iter().find(|f| f.id == b).expect("分支要在树里");
        assert_eq!(fa.parent, None);
        assert_eq!(fb.parent, Some(a));
        assert_eq!(fb.fork_block, Some(fork_point));

        // 块按时序（block_id 升序），归属各自正确。
        let ids: Vec<i64> = nodes.iter().map(|n| n.block.id).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted, "节点必须按时序排");

        let text_and_owner: Vec<(String, i64)> = nodes
            .iter()
            .map(|n| {
                let t = n
                    .block
                    .entries
                    .iter()
                    .map(|e| match e {
                        Entry::User { content } => content.clone(),
                        _ => String::new(),
                    })
                    .collect::<String>();
                (t, n.session)
            })
            .collect();
        assert!(
            text_and_owner.iter().any(|(t, o)| t == "A1" && *o == a),
            "A1 该归主干：{text_and_owner:?}"
        );
        assert!(
            text_and_owner.iter().any(|(t, o)| t == "B1" && *o == b),
            "B1 该归分支：{text_and_owner:?}"
        );
        // 主干自己后续的块（如果有）不会被分支顶替掉。
        assert!(tip_a < fork_point, "前置条件：A1 早于 A2");
    }

    /// 回退到**继承来的前缀**里（分叉点之前）：自己名下的块一块不剩，
    /// 会话就地重新生根到那一块的归属上——回退到哪，前缀就读到哪。
    #[test]
    fn pruning_into_an_inherited_prefix_re_roots_the_session() {
        let mut s = mem_store();
        let a = s.create_session("t", "/tmp").unwrap();
        s.append(a, &[user("A1"), user("A2"), user("A3")]).unwrap();
        let blocks = s.stored_blocks(a).unwrap();
        let (x, y) = (blocks[1].id, blocks[2].id);

        // 从 A3 岔出去，分支写自己的块。
        let b = s.fork_session(a, y).unwrap();
        s.append(b, &[user("B1")]).unwrap();
        assert_eq!(
            s.load_branch(b, None).unwrap().0.len(),
            4,
            "分叉后 B 看得到 A1 A2 A3 + B1"
        );

        // 回退到 A2——它在**继承的前缀**里，不是 B 自己的块。
        s.prune_session(b, x).unwrap();

        assert_eq!(s.stored_blocks(b).unwrap().len(), 0, "B 自己的块全没了");
        assert_eq!(
            user_texts(&s.load_branch(b, None).unwrap().0),
            vec!["A1", "A2"],
            "前缀读到回退点为止：A3 和 B1 都不该在"
        );
        let (_nodes, forks, _) = s.lineage(b).unwrap();
        assert!(
            forks.iter().any(|f| f.id == b),
            "行没换：B 还在树里，链没变长"
        );
    }

    /// 回退砍的是**整棵树**里回退点之后的一切：前几次回退岔出去的分支也一起
    /// 走，不能只砍自己名下那几块——否则 resume 一看，旧内容全在（用户报的
    /// 就是这一条）。
    #[test]
    fn pruning_takes_the_whole_family_with_it() {
        let mut s = mem_store();
        let a = s.create_session("t", "/tmp").unwrap();
        s.append(a, &[user("A1"), user("A2"), user("A3")]).unwrap();
        let blocks = s.stored_blocks(a).unwrap();
        let (x, y, z) = (blocks[0].id, blocks[1].id, blocks[2].id);

        // A3 上岔出一条 B，再在 B 上回退到 A2 岔出一条 C——就是连按回退的走法。
        let b = s.fork_session(a, z).unwrap();
        s.append(b, &[user("B1")]).unwrap();
        let c = s.fork_session(b, y).unwrap();
        s.append(c, &[user("C1")]).unwrap();

        // 站在 C 上回退到 A1。
        s.prune_session(c, x).unwrap();

        let a_blocks = s.stored_blocks(a).unwrap();
        assert_eq!(a_blocks.len(), 1, "主干只剩回退点之前那一块");
        assert_eq!(user_texts(&a_blocks[0].entries), vec!["A1"]);
        assert!(
            s.stored_blocks(b).unwrap().is_empty(),
            "被砍空的分支 B 的块也该没"
        );
        assert!(s.stored_blocks(c).unwrap().is_empty(), "C 自己的块没了");
        assert_eq!(
            user_texts(&s.load_branch(c, None).unwrap().0),
            vec!["A1"],
            "C 的前缀读到 A1 为止"
        );
        let (_nodes, forks, _) = s.lineage(c).unwrap();
        assert!(
            !forks.iter().any(|f| f.id == b),
            "被砍空的分支行该从树里消失：{forks:?}"
        );
        assert!(
            forks.iter().any(|f| f.id == c),
            "用户站着的会话必须还在：{forks:?}"
        );
    }

    /// 转录里所有 user 条目的文本（断言用）。
    fn user_texts(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .filter_map(|e| match e {
                Entry::User { content } => Some(content.clone()),
                _ => None,
            })
            .collect()
    }

    /// 回到历史节点时，树上要看得见**整棵树**：自己那条线 + 平行分支，
    /// 公共前缀只出现一次。
    ///
    /// 这是这轮修的主 bug——以前 `lineage` 从当前会话往上收祖先、再各自
    /// `WHERE session_id = ?1` 查块，于是从分支进去时前缀块查不到（它们存在
    /// 主干的 `session_id` 下），树上就只剩新写的那两块。
    #[test]
    fn lineage_from_a_branch_still_shows_the_whole_tree() {
        let mut s = mem_store();
        let a = s.create_session("t", "/tmp").unwrap();
        s.append(a, &[user("A1")]).unwrap();
        let fork_point = s.tip(a).unwrap().unwrap();
        s.append(a, &[user("A2")]).unwrap();

        let b = s.fork_session(a, fork_point).unwrap();
        s.append(b, &[user("B1")]).unwrap();

        // 从**分支**进去：主干的前缀也要在。
        let (nodes, _, current) = s.lineage(b).unwrap();
        let texts: Vec<String> = nodes
            .iter()
            .map(|n| {
                n.block
                    .entries
                    .iter()
                    .map(|e| match e {
                        Entry::User { content } => content.clone(),
                        _ => String::new(),
                    })
                    .collect()
            })
            .collect();
        assert_eq!(
            texts,
            vec!["A1".to_string(), "A2".to_string(), "B1".to_string()],
            "分支进去要看到主干"
        );
        // 每块只出现一次（`block_id` 是全局主键，不可能重）。
        let mut ids: Vec<i64> = nodes.iter().map(|n| n.block.id).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "块不能重复");
        // 归属各自正确：A1/A2 归主干，B1 归分支。
        let owner = |t: &str| {
            nodes
                .iter()
                .find(|n| matches!(&n.block.entries[0], Entry::User { content } if content == t))
                .map(|n| n.session)
        };
        assert_eq!(owner("A1"), Some(a));
        assert_eq!(owner("B1"), Some(b));
        // "我在哪" = 分支自己的 tip。
        assert_eq!(
            current,
            s.tip(b).unwrap().unwrap(),
            "current 要落在分支自己的最后一块"
        );
    }

    /// 从主干中间（不是叶子）分叉：新会话只继承**到那一块为止**的前缀，
    /// 主干后面写的块**不许**泄漏进分支。
    #[test]
    fn a_fork_only_inherits_the_prefix_up_to_its_point() {
        let mut s = mem_store();
        let a = s.create_session("t", "/tmp").unwrap();
        s.append(a, &[user("A1")]).unwrap();
        let mid = s.tip(a).unwrap().unwrap();
        // 主干在分叉点之后又写了两块。
        s.append(a, &[user("A2"), user("A3")]).unwrap();

        let b = s.fork_session(a, mid).unwrap();
        s.append(b, &[user("B1")]).unwrap();

        // 分支自己的线：只有 A1 + B1，没有 A2/A3。
        let own = texts_of(&s, b);
        assert_eq!(own, vec!["A1".to_string(), "B1".to_string()]);

        // 主干不受影响：三块都还在。
        let trunk = texts_of(&s, a);
        assert_eq!(
            trunk,
            vec!["A1".to_string(), "A2".to_string(), "A3".to_string()]
        );
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

    // ---- 时间戳：纯算术，不 fork ----

    /// `now_stamp` 的形状就是 picker 与 `display_name` 依赖的那一个：定长
    /// `YYYY-MM-DD HH:MM:SS`，`display_name` 按下标切片，差一位就是乱码。
    #[test]
    fn now_stamp_has_the_fixed_shape() {
        let s = now_stamp();
        assert_eq!(s.len(), 19, "定长 19：{s}");
        assert!(parse_stamp(&s).is_some(), "必须能被自家解析器读回：{s}");
        assert!(s.starts_with("20"), "本机时钟在 2000 年之后：{s}");
    }

    /// 已知纪元点上的历法算术（Hinnant 的 civil/days 互逆），覆盖闰年、
    /// 世纪年与 1970 之前 —— 手写日期代码的错都在这些边界上。
    #[test]
    fn civil_date_arithmetic_matches_known_epochs() {
        for (stamp, unix) in [
            ("1970-01-01 00:00:00", 0i64),
            ("1969-12-31 23:59:59", -1),
            ("2000-02-29 12:00:00", 951_825_600), // 世纪闰年
            ("2026-09-27 00:00:00", 1_790_467_200),
            ("2100-03-01 00:00:00", 4_107_542_400), // 2100 不是闰年
        ] {
            assert_eq!(
                parse_stamp(stamp),
                Some(unix),
                "{stamp} 的 Unix 秒数不对"
            );
            assert_eq!(stamp_from_unix(unix), stamp, "{unix} 该渲染回 {stamp}");
        }
    }

    /// 格式不对的戳必须返回 `None`：picker 宁可显示原文，也不显示一个算错的
    /// 年龄；而且解析器绝不能对短/畸形输入 panic（旧行、手改过的行）。
    #[test]
    fn malformed_stamps_are_rejected_not_panicked() {
        for bad in [
            "",
            "t",
            "2026-09-27",
            "2026-09-27 12:00",
            "2026-9-27 12:00:00",
            "2026-13-01 00:00:00",
            "2026-00-01 00:00:00",
            "2026-09-32 00:00:00",
            "2026-09-27 24:00:00",
            "2026-09-27 12:60:00",
            "2026x09x27 12:00:00",
            "2026/09/27 12:00:00",
        ] {
            assert_eq!(parse_stamp(bad), None, "该拒：{bad:?}");
            assert_eq!(age_seconds(bad), None, "age_seconds 也该拒：{bad:?}");
        }
        // 长度够但不是全数字的那一位，不能靠切片越界 panic。
        assert_eq!(parse_stamp("abcd-ef-gh ij:kl:mn"), None);
    }

    /// 年龄是"现在减戳"：未来的戳（时钟回拨）夹到 0，不返回负数。
    #[test]
    fn age_of_a_future_stamp_saturates_to_zero() {
        // +100 年，仍在 4 位年份的可表达范围内。
        let future = stamp_from_unix(4_102_444_800);
        assert_eq!(age_seconds(&future), Some(0), "未来戳不该报天文数字");
        // 真·过去：一整天的秒数必须原样反映出来（同一本地时区口径）。
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let day_ago = stamp_from_unix(now - 86_400 + local_offset_secs());
        assert_eq!(age_seconds(&day_ago), Some(86_400));
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

    /// 选择器把**同一根的分支合并成一行**：一个对话就是一行，不管它在
    /// 存储上被切成了几条会话。点进去要进**最新的那条分支**（用户上次停的地方）。
    #[test]
    fn the_picker_merges_a_family_into_one_row() {
        let mut s = mem_store();
        let a = s.create_session("2026-09-22 14:00:00", "/proj/a").unwrap();
        s.append(a, &[user("主干第一句")]).unwrap();
        let fork_point = s.tip(a).unwrap().unwrap();
        s.append(a, &[user("主干后面的话")]).unwrap();

        // 从主干分叉：这应该是"最新的分支"。
        let b = s.fork_session(a, fork_point).unwrap();
        s.append(b, &[user("分支第一句")]).unwrap();

        let rows = s.list_session_rows(None).unwrap();
        assert_eq!(rows.len(), 1, "一个根只该出一行：{rows:?}");
        let row = &rows[0];
        assert_eq!(row.meta.id, b, "点进去要进最新的那条分支");
        assert_eq!(row.root, a, "根是原始主干");
        // 体积是所有分支加起来的。
        let own = |id: i64| -> i64 {
            s.conn
                .query_row("SELECT bytes FROM sessions WHERE id = ?1", [id], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(row.bytes, own(a) + own(b), "体积要合并");
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
        let compat = crate::server::ai::compat::resolve("http://127.0.0.1:1/v1", "m", None);
        let wire = serde_json::to_string(&crate::server::ai::assemble::messages_to_wire(
            &ctx.messages,
            &compat,
        ))
        .unwrap();
        assert!(wire.contains("ok"), "{wire}");
        assert!(
            !wire.contains("先想"),
            "没有宿主要求时推理不上线：{wire}"
        );
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
