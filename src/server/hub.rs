//! Multi-session host: one process, many conversations, no shared state.
//!
//! The "one server" in "one server, two projects" is this type. It owns the
//! database file and a registry of open sessions; each session gets **its own
//! SQLite connection** to that file, its own transcript, its own working
//! directory, its own model, its own streaming slots and its own event
//! channel. Isolation is structural, not a discipline someone has to keep:
//! every table is keyed by `session_id` and no code path can reach across.
//!
//! ```text
//!              ┌──────────── SessionHub ────────────┐
//!   open_new → │  id 1: Session + Store + rx        │ ← turn thread (project A)
//!   open_new → │  id 2: Session + Store + rx        │ ← turn thread (project B)
//!              └─────────────┬──────────────────────┘
//!                            │  sessions.db3 (WAL: readers never block, writers queue)
//! ```
//!
//! Concurrency: turns already run on their own threads (`turn::spawn_turn`),
//! so two sessions can be mid-generation at the same time. The hub itself is
//! *not* threaded — it is the single owner of every session and is driven from
//! one thread (today: the UI's), exactly like a session is. SQLite serializes
//! the writers; nothing else is shared, so nothing else needs a lock.
//!
//! The hub never learns which surface is driving it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use crate::server::ai::client::Client;
use crate::server::ai::config::{BrowserConfig, Cost, StreamMode, ToolsConfig};
use crate::server::ai::types::{Context as ChatContext, Message};
use crate::server::events::{Change, RunState, SessionEvent};
use crate::server::session::{Session, SessionState};
use crate::server::store::Store;

/// Everything a new (or resumed) session needs that is not the hub's business.
///
/// Not a grab-bag: each field is a per-session decision — which gateway, which
/// system prompt, which tool roster, which directory. Two sessions may differ
/// in all of them at once, which is the point.
#[derive(Clone)]
pub struct SessionSpec {
    /// Provider connection (endpoint + key + model) for **this** session.
    pub client: Client,
    /// System prompt verbatim. Stored with every round it is used for, so a
    /// resumed session replays the prompt it actually had, not today's.
    pub system_prompt: String,
    pub max_tokens: u32,
    pub cost: Cost,
    /// Context window of the current model (models.yml `contextWindow`),
    /// the usage gauge's denominator. Server-owned metadata: the front end
    /// never reads models.yml. 0 = unknown (gauge hides).
    pub context_window: u64,
    /// Statusline display name for the model (models.yml `name`), falling
    /// back to the id when unnamed. Display-only; the id stays in `Client`.
    pub model_name: String,
    /// 服务商名（models.yml 的段名）。`/usage` 的账单认人用：同名模型可以
    /// 挂在不同服务商名下，光有 id 分不出来。
    pub model_provider: String,
    /// 计价货币（models.yml 的 `currency`）。状态栏的金额和 `/usage` 的账单
    /// 都按它写符号——同一个数在 `¥` 和 `$` 下不是一回事。
    pub currency: crate::server::ai::config::Currency,
    /// Working directory. A resume prefers the stored one (see [`SessionHub::resume`]).
    pub cwd: PathBuf,
    /// Tool allow-list (`None` = all).
    pub tool_filter: Option<Vec<String>>,
    /// Where the profile's roster may be re-read from (see
    /// `tools.reloadOnCompaction`). `None` = the roster is fixed for the
    /// session's life.
    pub roster_source: Option<std::sync::Arc<dyn Fn() -> Option<Vec<String>> + Send + Sync>>,
    pub tools: ToolsConfig,
    /// Compaction knobs (`config.yaml → compact:`), for `/compact`.
    pub compact: crate::server::ai::config::CompactConfig,
    /// The active profile's `profiles/<name>/compact.md`, when it has one.
    /// Read fresh at each compaction; `None` falls through to
    /// `compact.instruction_file`, then the built-in instruction.
    pub compact_instruction: Option<PathBuf>,
    /// What session-scoped commands may reach outside the session
    /// (`/switch`, `/profile`, `/model`). Empty = not wired; the commands say
    /// so rather than pretend. See [`crate::server::commands::CommandEnv`].
    pub commands: crate::server::commands::CommandEnv,
    pub browser: BrowserConfig,
    /// How the turn's **text** reaches subscribers (config.yaml → `app.streaming`,
    /// or `MYPI_STREAM_MODE`). Entries are shipped block by block either way;
    /// this only picks whether a block's text arrives as it is typed
    /// ([`StreamMode::Immediate`]) or whole once that block's text is done
    /// ([`StreamMode::Buffered`]). See [`StreamMode`].
    pub stream_mode: StreamMode,
}

struct OpenSession {
    session: Session,
    // This session's own protocol channel: the turn thread it spawned writes
    // here and nowhere else, so an event can never land in another session.
    rx: Receiver<SessionEvent>,
}

pub struct SessionHub {
    db: PathBuf,
    sessions: BTreeMap<i64, OpenSession>,
}

static PRUNE_ON_REWIND: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// 回退到旧节点时是否**剪掉**后面的块（`config.yaml` → `tui.PruneOnRewind`）。
///
/// 默认剪：回退是后悔药，被抛下的内容就该扔掉，会话链不该越用越长。
/// 置 false 则每次回退开一条新分支（一行 INSERT，不拷数据），之后任何节点
/// 都还能回去。
pub fn prune_on_rewind() -> bool {
    *PRUNE_ON_REWIND.get().unwrap_or(&true)
}

/// 钉住开关（启动时，从配置）。首次调用生效——同
/// [`crate::server::entry::set_empty_reply`]。
pub fn set_prune_on_rewind(on: bool) {
    let _ = PRUNE_ON_REWIND.set(on);
}

impl SessionHub {
    pub fn new(db: PathBuf) -> Self {
        Self {
            db,
            sessions: BTreeMap::new(),
        }
    }

    pub fn db(&self) -> &Path {
        &self.db
    }

    /// Ids of the open sessions, ascending.
    pub fn ids(&self) -> Vec<i64> {
        self.sessions.keys().copied().collect()
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    pub fn get(&self, id: i64) -> Option<&Session> {
        self.sessions.get(&id).map(|s| &s.session)
    }

    pub fn get_mut(&mut self, id: i64) -> Option<&mut Session> {
        self.sessions.get_mut(&id).map(|s| &mut s.session)
    }

    /// Coarse state of one session, for clients that render no transcript.
    pub fn status(&self, id: i64) -> Option<RunState> {
        self.get(id).map(|s| s.status())
    }

    /// Create a fresh session: new row, empty transcript, own connection.
    ///
    /// Fails when the database cannot be opened — a hub exists to persist
    /// several conversations, so a silently in-memory session would defeat it.
    pub fn open_new(&mut self, spec: SessionSpec) -> anyhow::Result<i64> {
        let store = Store::open(&self.db)?;
        let chat = ChatContext::new().push(Message::System {
            content: spec.system_prompt.clone(),
        });
        let (mut session, rx) = self.assemble(SessionState::new(Some(store)), spec, chat);
        let cwd = session.cwd();
        let id = session
            .ensure_session(&cwd)
            .ok_or_else(|| anyhow::anyhow!("cannot create the session (database unavailable)"))?;
        self.sessions.insert(id, OpenSession { session, rx });
        Ok(id)
    }

    /// Reopen a stored session: load its transcript and cwd, rebuild the model
    /// context from the entries (the same reconstruction a live run uses), and
    /// adopt the given client/system prompt for **future** rounds.
    ///
    /// The stored working directory wins over `spec.cwd`: a resumed session
    /// belongs where it was, not where the process happens to be.
    /// 惰性历史的尾巴长度：够画满第一屏（一屏 ~40 行，条目有大有小）。
    pub(crate) const HISTORY_TAIL: usize = 400;

    pub fn resume(&mut self, id: i64, spec: SessionSpec) -> anyhow::Result<i64> {
        if self.sessions.contains_key(&id) {
            return Ok(id);
        }
        let store = Store::open(&self.db)?;
        let meta = store.session(id)?;
        // **只读尾巴**：第一帧只要一屏，整读在 32k 块上要 260 ms（§30）。
        // 其余由 daemon 按前端点名的块区间补（`ServerMsg::OlderBlocks`）。
        let (tail, has_more) = store.load_tail(id, Self::HISTORY_TAIL)?;
        let entries: Vec<crate::server::entry::Entry> =
            tail.into_iter().flat_map(|b| b.entries).collect();
        // Name and cwd come from storage, not from the caller: a resumed
        // session belongs where it was, under the name it had.
        let name = store.effective_name(id)?;
        let cwd = meta
            .cwd
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| spec.cwd.clone());
        // The model context is rebuilt from the entries by the same function a
        // live turn uses, so a resumed session sees the conversation it had —
        // **unless** only the tail is in hand: then a context built from it
        // would silently drop everything older, so it waits for the full
        // transcript (`Session::ensure_full_history`, first turn).
        let chat = if has_more {
            crate::server::turn::entries_to_context(&spec.system_prompt, &[])
        } else {
            crate::server::turn::entries_to_context(&spec.system_prompt, &entries)
        };
        let (mut session, rx) = self.assemble(SessionState::new(Some(store)), spec, chat);
        let _ = session.adopt_session(id, entries, name);
        session.set_history_pending(has_more);
        session.set_cwd(cwd);
        // 会话行里记着上一次 `/switch` 到的模型：换回去。这是 `/switch` 落盘的
        // 全部意义——接着聊不用再敲一遍。
        session.restore_model();
        // 账本也要接上：花钱是事实，不该因为重启就归零。
        session.restore_spend();
        self.sessions.insert(id, OpenSession { session, rx });
        Ok(id)
    }

    /// Build a session from a spec + the context the hub assembled.
    fn assemble(
        &self,
        state: SessionState,
        spec: SessionSpec,
        chat: ChatContext,
    ) -> (Session, Receiver<SessionEvent>) {
        let (mut session, rx) = Session::new(
            state,
            spec.client,
            chat,
            spec.max_tokens,
            spec.cost,
            spec.context_window,
            spec.model_name,
            spec.model_provider,
            spec.currency,
            spec.cwd,
            spec.tool_filter,
            spec.tools,
            spec.browser,
            Some(self.db.clone()),
        );
        // 交付节拍来自配置（`config.yaml → app.streaming`）：会话本身不知道
        // 配置从哪来，spec 是唯一入口。
        session.set_stream_mode(spec.stream_mode);
        // `/compact` 的旋钮来自配置：不接的话它会**静默**用默认值——
        // 用户改了 config.yaml 却没生效，最难看的一种失败。
        session = session.with_compact(spec.compact.clone());
        // The profile may carry its own compaction instruction. Taken from the
        // spec at assembly, so a resume keeps the profile it was started with.
        // `/profile` swaps it later (see `Session::apply_profile`).
        session = session.with_compact_instruction(spec.compact_instruction.clone());
        session = session.with_commands(spec.commands.clone());
        if let Some(src) = spec.roster_source.clone() {
            session = session.with_roster_source(src);
        }
        (session, rx)
    }

    /// Start a round in **one** session. False when that session is already
    /// streaming — the other sessions are unaffected either way.
    pub fn submit(&mut self, id: i64, text: &str) -> bool {
        match self.sessions.get_mut(&id) {
            Some(o) => o.session.submit(text),
            None => false,
        }
    }

    /// Drain one session's channel into its state, returning what changed.
    pub fn drain(&mut self, id: i64) -> Vec<Change> {
        let Some(o) = self.sessions.get_mut(&id) else {
            return Vec::new();
        };
        let mut changes = Vec::new();
        while let Ok(ev) = o.rx.try_recv() {
            changes.push(o.session.ingest(ev));
        }
        changes
    }

    /// Drain every session. Each entry is tagged with its session id, so a
    /// caller routes a repaint per conversation and never mixes two.
    ///
    /// Sessions with nothing new are omitted (an idle session is not news).
    pub fn drain_all(&mut self) -> Vec<(i64, Vec<Change>)> {
        let ids = self.ids();
        let mut out = Vec::new();
        for id in ids {
            let changes = self.drain(id);
            if !changes.is_empty() {
                out.push((id, changes));
            }
        }
        out
    }

    /// Forget a session (its stored data stays on disk).
    pub fn close(&mut self, id: i64) -> bool {
        self.sessions.remove(&id).is_some()
    }

    // ---- queries & controls that live below the open sessions ----

    /// Every stored session (picker / `mypi sessions`). Reads the database
    /// directly: closed sessions are sessions too.
    pub fn list_sessions(&self) -> anyhow::Result<Vec<crate::server::store::SessionMeta>> {
        Store::open(&self.db)?.list_sessions()
    }

    /// Resume-picker rows (`/resume`): metadata + preview + size, optionally
    /// narrowed to one project directory. Read-only, one query.
    pub fn list_session_rows(
        &self,
        under: Option<&std::path::Path>,
    ) -> anyhow::Result<Vec<crate::server::store::SessionRow>> {
        Store::open(&self.db)?.list_session_rows(under)
    }

    /// Delete a stored session and everything hanging off it.
    ///
    /// Refused while its turn is **running**: the turn thread appends entries
    /// as it goes, and deleting the row underneath it leaves the writer
    /// pointing at nothing. Open-but-idle is fine — that is the normal case in
    /// the resume picker, where you just came from the session you want gone.
    pub fn delete_session(&mut self, id: i64) -> anyhow::Result<bool> {
        if let Some(s) = self.sessions.get(&id)
            && s.session.busy()
        {
            anyhow::bail!("session {id} is running a turn; interrupt it before deleting");
        }
        self.close(id);
        Store::open(&self.db)?.delete_session(id)
    }

    /// One session's stored round headers.
    pub fn rounds(&self, id: i64) -> anyhow::Result<Vec<crate::server::store::RoundRow>> {
        Store::open(&self.db)?.rounds(id)
    }

    /// Rebuild one stored round's request from the database (read-only).
    pub fn replay(&self, id: i64, round: i64) -> Result<crate::server::session::Replay, String> {
        // Replay reads the store of the *stored* session: open a session
        // facade just for this query if it is not live. The facade needs a
        // spec-shaped client only for live rounds, which replay never touches,
        // so the in-memory state with the db path suffices.
        if let Some(s) = self.sessions.get(&id) {
            return s.session.replay_round(id, round);
        }
        let store = Store::open(&self.db).map_err(|e| e.to_string())?;
        let st = crate::server::session::SessionState::new(Some(store));
        st.replay_round(id, round)
    }

    /// Interrupt the running round of one session. No-op when idle.
    pub fn interrupt(&self, id: i64) {
        if let Some(s) = self.sessions.get(&id) {
            s.session.interrupt_turn();
        }
    }

    /// 从 `parent` 的 `block` 处**分叉**：新开一条会话，继承 `<= block` 的
    /// 前缀（`Store::fork_session`，一行 INSERT，不拷数据）。
    ///
    /// 这是 server 侧对节点树**唯一**的写操作。谁调、什么时候调是前端的事
    /// （"不发消息不建会话"由前端守，见 `ClientMsg::ForkAt` 的注释）。
    pub fn fork_at(&mut self, parent: i64, block: i64) -> anyhow::Result<i64> {
        let mut store = Store::open(&self.db)?;
        store.fork_session(parent, block)
    }

    /// 把一个**已经开着的**会话**回退**到 `block` 上，返回它的会话行 id。
    ///
    /// 回退不是"开一个新会话"——内存里那份 `Session`（转录、cwd、模型上下文、
    /// 运行时权柄）全部留着，只把它指到 `block` 为止的历史上。这样回到旧节点
    /// 发消息**不会**撞上"没打开/忙碌"，也不会把中间攒下的运行时状态扔掉。
    ///
    /// 被抛下的那段历史怎么落，由 `tui.PruneOnRewind` 定：
    ///
    /// - **剪**（默认）：[`Store::prune_session`] 就地截断——`block` 之后自己
    ///   名下的块连同回合记录一起删掉，同一行内挪 tip。回退不可逆，也不会让
    ///   分支链变长。
    /// - **留**：[`Store::fork_session`] 开一条新分支（一行 INSERT，不拷数据），
    ///   旧内容原样留着，之后任何节点都还能回去。
    ///
    /// 两条路都要接管在跑的那一轮：转录换成**选中的那块为止的前缀**（含），
    /// 否则新历史会带着分叉点之后那些本该被抛下的块继续跑；上一轮剩下的收尾
    /// 事件也不该写进来（`abandon_turn` 把事件通道和上下文副本一起换掉）。
    pub fn fork_in_place(&mut self, id: i64, block: i64) -> anyhow::Result<i64> {
        self.rewind(id, block, prune_on_rewind())
    }

    /// [`Self::fork_in_place`] 的本体。`prune` 走参数而不是读全局：两条路
    /// 都要能测，而开关是进程级 `OnceLock`，测试里只设得上一次。
    fn rewind(&mut self, id: i64, block: i64, prune: bool) -> anyhow::Result<i64> {
        let mut store = Store::open(&self.db)?;
        let new_id = if prune {
            // 被整条砍掉的会话可能还开在内存里（用户切走过、但没关）。它们的
            // 块已经没了，句柄留着只会往一个空壳里写。
            for gone in store.prune_session(id, block)? {
                self.sessions.remove(&gone);
            }
            id
        } else {
            store.fork_session(id, block)?
        };
        // 前缀 = 从会话根到 `block`（含）的全部条目，跨分叉点拼。
        let (entries, _missing) = store.load_branch(new_id, Some(block))?;

        let mut open = self
            .sessions
            .remove(&id)
            .ok_or_else(|| anyhow::anyhow!("session {id} is not in memory; cannot rewind"))?;
        // 先换 tip（转录变成新前缀），再抛下在跑的那一轮：上一轮属于被抛下的
        // 分支——它不该继续把新分支堵成"忙"，也不该把半截回复写进新分支。
        // 上下文副本也跟着新前缀重建（`abandon_turn` 内部做）。
        open.session.retarget(new_id, entries);
        open.rx = open.session.abandon_turn();
        self.sessions.insert(new_id, open);
        Ok(new_id)
    }

    /// 会话树（节点树页面）：整条 fork 线，块按时序排，附每条 fork 的挂载点。
    ///
    /// 只读。读的是 `sessions.parent_id` / `fork_block_id` 和 `blocks.session_id`
    /// ——都是写块时就落好的既有列，不做任何推导。
    pub fn tree(
        &self,
        id: i64,
    ) -> Result<
        (
            Vec<crate::server::wire::TreeNode>,
            Vec<crate::server::wire::ForkInfo>,
            usize,
        ),
        String,
    > {
        let store = Store::open(&self.db).map_err(|e| e.to_string())?;
        let (nodes, forks, current_block) =
            store.lineage(id).map_err(|e| format!("{e:#}"))?;

        // 逐**条目**展开：一块可能出两行（工具往返），也可能一行不出
        // （一整块都是状态条目）。
        let mut out = Vec::new();
        // `current` 是**行**的位置，不是块的位置：一块能出两行。取"我停在哪块"
        // 那一块**最后**出现的行——同块两行（call/result）时停在下面那行更自然。
        let mut current = 0usize;
        for n in nodes {
            for e in &n.block.entries {
                if let Some((kind, summary)) = crate::server::wire::summarize_entry_kind(e) {
                    if n.block.id == current_block {
                        current = out.len();
                    }
                    out.push(crate::server::wire::TreeNode {
                        id: n.block.id,
                        session: n.session,
                        kind,
                        summary,
                    });
                }
            }
        }
        let nodes = out;
        let forks = forks
            .into_iter()
            .map(|f| crate::server::wire::ForkInfo {
                id: f.id,
                parent: f.parent,
                fork_block: f.fork_block,
            })
            .collect();
        Ok((nodes, forks, current))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::ai::client::Client;
    use crate::server::entry::Entry;
    use crate::server::test_gateway::{chat_json, fake_gateway, sse};

    fn tmp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mypi-hub-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn spec(base: &str, model: &str, system: &str, cwd: &Path) -> SessionSpec {
        SessionSpec {
            client: Client::new(base, "k", model),
            system_prompt: system.into(),
            max_tokens: 4096,
            cost: Cost::default(),
            currency: crate::server::ai::config::Currency::Cny,
            context_window: 0,
            model_name: model.to_string(),
            model_provider: "test".into(),
            cwd: cwd.to_path_buf(),
            tool_filter: None,
            roster_source: None,
            tools: ToolsConfig::default(),
            compact: Default::default(),
            compact_instruction: None,
            commands: Default::default(),
            browser: BrowserConfig::default(),
            stream_mode: StreamMode::default(),
        }
    }

    /// 一次历史都没跑过的新会话，也得给用量表一个起点（开场白的字符折算）。
    /// 「借历史实测值」那条路走 `min_prompt_tokens`，由
    /// `store::tests::the_smallest_recorded_prompt_is_borrowed` 锁住。
    #[test]
    fn a_fresh_session_estimates_from_its_opening() {
        let dir = tmp("estimate-fresh");
        let mut hub = SessionHub::new(dir.join("sessions.db"));
        let id = hub
            .open_new(spec("http://127.0.0.1:1/v1", "m", "S", &dir))
            .unwrap();
        let n = hub.get(id).unwrap().context_estimate();
        assert!(n > 0, "系统提示词加工具手册再短也得出个数，不能是 0");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 同一条会话算两次给同一个数（缓存住，状态帧每跳都可能来问）。
    #[test]
    fn the_estimate_is_computed_once() {
        let dir = tmp("estimate-cached");
        let mut hub = SessionHub::new(dir.join("sessions.db"));
        let id = hub
            .open_new(spec("http://127.0.0.1:1/v1", "m", "S", &dir))
            .unwrap();
        let s = hub.get(id).unwrap();
        assert_eq!(s.context_estimate(), s.context_estimate());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `/switch` 的验收：**用户看得到的东西**要跟着换。
    ///
    /// 状态栏的模型名、上下文窗口（量表分母）和 client 里的 id 是一套的：
    /// 只换一半比不换更糟——量表按旧窗口算，屏幕上写着新名字。
    #[test]
    fn switching_models_moves_client_and_metadata_together() {
        use crate::server::commands::{CommandEnv, ModelSwitch};
        let dir = tmp("switch-model");
        let mut hub = SessionHub::new(dir.join("sessions.db"));
        let mut sp = spec("http://127.0.0.1:1/v1", "model-a", "S", &dir);
        sp.commands = CommandEnv {
            resolve_model: Some(std::sync::Arc::new(|id: &str| {
                if id == "fake:model-b" {
                    Ok(ModelSwitch {
                        base_url: "http://127.0.0.1:2/v1".into(),
                        api_key: "sk-b".into(),
                        model_id: "model-b".into(),
                        provider: "fake".into(),
                        name: "FAKE-B".into(),
                        context_window: 999,
                        max_tokens: 1234,
                        compat: None,
                        cost: Cost::default(),
                        currency: crate::server::ai::config::Currency::Cny,
                    })
                } else {
                    Err(format!("未知模型 {id}"))
                }
            })),
            ..Default::default()
        };
        let id = hub.open_new(sp).unwrap();
        let cmd = crate::server::commands::lookup("/switch").expect("/switch 在表里");

        hub.get_mut(id)
            .unwrap()
            .run_command(cmd, "fake:model-b")
            .unwrap();
        let s = hub.get(id).unwrap();
        assert_eq!(s.model(), "model-b", "client 换到新模型");
        assert_eq!(s.model_name(), "FAKE-B", "状态栏显示名");
        assert_eq!(s.context_window(), 999, "量表分母");

        // 未知 id：命令失败，且**什么都没换**。
        let err = hub
            .get_mut(id)
            .unwrap()
            .run_command(cmd, "fake:nope")
            .unwrap_err();
        assert!(format!("{err:#}").contains("未知模型"), "{err:#}");
        let s = hub.get(id).unwrap();
        assert_eq!(s.model(), "model-b", "被拒的命令不许动 client");
        assert_eq!(s.model_name(), "FAKE-B", "也不许动显示名");

        // 没接解析器时必须**明说**，不能装作切过了。
        let bare = tmp("switch-bare");
        let mut hub2 = SessionHub::new(bare.join("sessions.db"));
        let id2 = hub2.open_new(spec("http://127.0.0.1:1/v1", "model-a", "S", &bare)).unwrap();
        let err = hub2
            .get_mut(id2)
            .unwrap()
            .run_command(cmd, "fake:model-b")
            .unwrap_err();
        assert!(format!("{err:#}").contains("not wired"), "{err:#}");
    }

    /// `/switch` 要落盘：切过模型的会话关掉再开回来，模型自己回来——接着聊
    /// 不该再敲一遍。存的是全局 id，endpoint 和密钥现从配置解析（不落盘）。
    #[test]
    fn a_session_remembers_the_model_it_was_switched_to() {
        use crate::server::commands::{CommandEnv, ModelSwitch};
        let dir = tmp("switch-persist");
        let mut hub = SessionHub::new(dir.join("sessions.db"));
        let make_spec = || {
            let mut sp = spec("http://127.0.0.1:1/v1", "model-a", "S", &dir);
            sp.commands = CommandEnv {
                resolve_model: Some(std::sync::Arc::new(|id: &str| {
                    if id == "fake:model-b" {
                        Ok(ModelSwitch {
                            base_url: "http://127.0.0.1:2/v1".into(),
                            api_key: "sk-b".into(),
                            model_id: "model-b".into(),
                            provider: "fake".into(),
                            name: "FAKE-B".into(),
                            context_window: 999,
                            max_tokens: 1234,
                            compat: None,
                            cost: Cost::default(),
                            currency: crate::server::ai::config::Currency::Cny,
                        })
                    } else {
                        Err(format!("未知模型 {id}"))
                    }
                })),
                ..Default::default()
            };
            sp
        };
        let id = hub.open_new(make_spec()).unwrap();
        let cmd = crate::server::commands::lookup("/switch").expect("/switch 在表里");
        hub.get_mut(id)
            .unwrap()
            .run_command(cmd, "fake:model-b")
            .unwrap();
        assert_eq!(hub.get(id).unwrap().model_name(), "FAKE-B");
        hub.close(id);

        // 关掉再开回来：模型跟着会话走。
        hub.resume(id, make_spec()).unwrap();
        let s = hub.get(id).unwrap();
        assert_eq!(s.model(), "model-b", "resume 换回会话切到的模型");
        assert_eq!(s.model_name(), "FAKE-B", "显示名一起回来");
        assert_eq!(s.context_window(), 999, "量表分母一起回来");

        // 没切过的会话：resume 之后还是配置里的默认模型。
        let other = hub.open_new(make_spec()).unwrap();
        hub.close(other);
        hub.resume(other, make_spec()).unwrap();
        assert_eq!(
            hub.get(other).unwrap().model(),
            "model-a",
            "没切过就用默认"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 会话记着的模型解析不出来时**卡住这个会话**，绝不静默换成默认模型——
    /// 上下文一长，用错的模型接着跑是真金白银。`/switch` 到能用的模型就解卡。
    #[test]
    fn an_unresolvable_session_model_blocks_until_switched() {
        use crate::server::commands::{CommandEnv, ModelSwitch};
        let dir = tmp("model-blocked");
        let mut hub = SessionHub::new(dir.join("sessions.db"));
        let resolver = |b_available: bool| {
            Some(std::sync::Arc::new(move |id: &str| match id {
                "fake:model-b" if b_available => Ok(ModelSwitch {
                    base_url: "http://127.0.0.1:2/v1".into(),
                    api_key: "sk-b".into(),
                    model_id: "model-b".into(),
                    provider: "fake".into(),
                    name: "FAKE-B".into(),
                    context_window: 999,
                    max_tokens: 1234,
                    compat: None,
                    cost: Cost::default(),
                    currency: crate::server::ai::config::Currency::Cny,
                }),
                "fake:model-c" => Ok(ModelSwitch {
                    base_url: "http://127.0.0.1:3/v1".into(),
                    api_key: "sk-c".into(),
                    model_id: "model-c".into(),
                    provider: "fake".into(),
                    name: "FAKE-C".into(),
                    context_window: 111,
                    max_tokens: 55,
                    compat: None,
                    cost: Cost::default(),
                    currency: crate::server::ai::config::Currency::Cny,
                }),
                other => Err(format!("未知模型 {other}")),
            }) as crate::server::commands::ModelResolver)
        };
        let mut sp = spec("http://127.0.0.1:1/v1", "model-a", "S", &dir);
        sp.commands = CommandEnv {
            resolve_model: resolver(true),
            ..Default::default()
        };
        let id = hub.open_new(sp).unwrap();
        let cmd = crate::server::commands::lookup("/switch").expect("/switch 在表里");
        hub.get_mut(id)
            .unwrap()
            .run_command(cmd, "fake:model-b")
            .unwrap();
        assert!(hub.get(id).unwrap().model_block().is_none(), "刚切完能用");
        hub.close(id);

        // 配置变了：那个模型解析不出来。
        let mut sp2 = spec("http://127.0.0.1:1/v1", "model-a", "S", &dir);
        sp2.commands = CommandEnv {
            resolve_model: resolver(false),
            ..Default::default()
        };
        hub.resume(id, sp2).unwrap();
        let s = hub.get(id).unwrap();
        assert!(
            s.model_block().is_some(),
            "换不回去就得卡住，不能装作没事"
        );
        assert_eq!(
            s.model(),
            "model-a",
            "更不许静默拿默认模型顶上（spec 里那个才是默认）"
        );

        // `/switch` 到还能解析出来的那个 → 解卡，模型真的换了。
        let s = hub.get_mut(id).unwrap();
        let cmd = crate::server::commands::lookup("/switch").expect("/switch 在表里");
        s.run_command(cmd, "fake:model-c").unwrap();
        assert!(s.model_block().is_none(), "换到能用的模型就解卡");
        assert_eq!(s.model_name(), "FAKE-C");
        assert_eq!(s.model(), "model-c");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// resume 之后状态栏的金额**接着算**：库里按轮记着 token，重新打开时拿
    /// 当前价格表重算一遍。不接的话每次重启金额都从零开始——用户看到的是
    /// "这个组件坏了"。
    #[test]
    fn a_resumed_session_carries_its_spending_forward() {
        use crate::server::commands::{CommandEnv, ModelSwitch};
        use crate::server::events::SessionEvent;
        let dir = tmp("spend-restore");
        let price = Cost {
            input: 1.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
        };
        // 模型单价 1/M：一百万输入 token 正好一块钱，断言不用容差。
        let resolver = std::sync::Arc::new(move |_: &str| {
            Ok(ModelSwitch {
                base_url: "http://127.0.0.1:1/v1".into(),
                api_key: "k".into(),
                model_id: "m".into(),
                provider: "test".into(),
                name: "M".into(),
                context_window: 1_000_000,
                max_tokens: 4096,
                compat: None,
                cost: price,
                currency: crate::server::ai::config::Currency::Cny,
            })
        });
        let mk = || {
            let mut sp = spec("http://127.0.0.1:1/v1", "m", "S", &dir);
            sp.cost = price;
            sp.commands = CommandEnv {
                resolve_model: Some(resolver.clone()),
                ..Default::default()
            };
            sp
        };

        let mut hub = SessionHub::new(dir.join("sessions.db"));
        let id = hub.open_new(mk()).unwrap();
        {
            let s = hub.get_mut(id).unwrap();
            // 一轮带请求头的回合：账要落到 `turns` 上（没有请求头的那条路
            // 根本不写账本，也就无从恢复）。
            s.ingest(SessionEvent::RequestMeta {
                model: "m".into(),
                protocol: "openai-chat-completions".into(),
                base_url: "http://127.0.0.1:1/v1".into(),
                system: "S".into(),
                tools_json: "[]".into(),
                max_tokens: 4096,
                compat_json: None,
            });
            s.ingest(SessionEvent::Delta("答".into()));
            s.ingest(SessionEvent::TurnDone(
                crate::server::ai::types::Usage {
                    prompt_tokens: 1_000_000,
                    completion_tokens: 0,
                    total_tokens: 1_000_000,
                    cached_tokens: None,
                    reasoning_tokens: None,
                },
                crate::server::ai::types::StopReason::Stop,
            ));
        }
        assert!((hub.get(id).unwrap().spend().0 - 1.0).abs() < 1e-9, "跑完这一轮就是一块钱");
        hub.close(id);

        hub.resume(id, mk()).unwrap();
        let spend = hub.get(id).unwrap().spend().0;
        assert!(
            (spend - 1.0).abs() < 1e-9,
            "重新打开后金额必须接着算，不能归零（现在是 {spend}）"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `/profile` 的两半：下一次请求带的系统提示词，和工具名册。
    #[test]
    fn switching_profile_swaps_prompt_and_roster() {
        use crate::server::commands::CommandEnv;
        let dir = tmp("switch-profile");
        let mut hub = SessionHub::new(dir.join("sessions.db"));
        let mut sp = spec("http://127.0.0.1:1/v1", "m", "原来的提示词", &dir);
        sp.tool_filter = Some(vec!["read".into()]);
        sp.commands = CommandEnv {
            resolve_profile: Some(std::sync::Arc::new(|name: &str| {
                if name == "审阅" {
                    Ok(crate::server::profile::ResolvedProfile {
                        system: "你现在是审阅者".to_string(),
                        tools: Some(vec!["read".into(), "grep".into()]),
                        compact_instruction: None,
                    })
                } else {
                    Err(format!("未知 profile {name}"))
                }
            })),
            ..Default::default()
        };
        let id = hub.open_new(sp).unwrap();
        assert_eq!(hub.get(id).unwrap().system_prompt(), "原来的提示词");

        let cmd = crate::server::commands::lookup("/profile").expect("/profile 在表里");
        hub.get_mut(id).unwrap().run_command(cmd, "审阅").unwrap();
        let s = hub.get(id).unwrap();
        assert_eq!(s.system_prompt(), "你现在是审阅者", "下一次请求就带这个");
        assert_eq!(
            s.tool_roster().unwrap(),
            &vec!["read".to_string(), "grep".to_string()],
            "名册跟着换"
        );

        let err = hub.get_mut(id).unwrap().run_command(cmd, "没有这个").unwrap_err();
        assert!(format!("{err:#}").contains("未知 profile"), "{err:#}");
        assert_eq!(hub.get(id).unwrap().system_prompt(), "你现在是审阅者", "失败不动状态");
    }

    /// `/model` 的两个落脚点：本会话元数据（立刻），配置文件（下次启动）。
    ///
    /// 顺序也是契约：**先解析再落盘**。id 不存在时必须一个字节都不写——
    /// 这条命令最容易造成的伤害就是把用户的下一份配置写坏。
    #[test]
    fn model_persists_the_default_only_after_it_resolved() {
        use crate::server::commands::{CommandEnv, ModelSwitch};
        use std::sync::{Arc, Mutex};
        let dir = tmp("set-default-model");
        let mut hub = SessionHub::new(dir.join("sessions.db"));
        let written: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let mut sp = spec("http://127.0.0.1:1/v1", "model-a", "S", &dir);
        sp.commands = CommandEnv {
            resolve_model: Some(Arc::new(|id: &str| {
                if id == "fake:model-b" {
                    Ok(ModelSwitch {
                        base_url: "http://127.0.0.1:2/v1".into(),
                        api_key: "k".into(),
                        model_id: "model-b".into(),
                        provider: "fake".into(),
                        name: "FAKE-B".into(),
                        context_window: 42,
                        max_tokens: 1,
                        compat: None,
                        cost: Cost::default(),
                        currency: crate::server::ai::config::Currency::Cny,
                    })
                } else {
                    Err(format!("未知模型 {id}"))
                }
            })),
            set_default_model: Some({
                let written = written.clone();
                Arc::new(move |id: &str| {
                    written.lock().unwrap().push(id.to_string());
                    Ok("/tmp/config.yaml".to_string())
                })
            }),
            ..Default::default()
        };
        let id = hub.open_new(sp).unwrap();
        let cmd = crate::server::commands::lookup("/model").expect("/model 在表里");

        hub.get_mut(id).unwrap().run_command(cmd, "fake:model-b").unwrap();
        assert_eq!(
            written.lock().unwrap().as_slice(),
            ["fake:model-b"],
            "落盘一次，写的就是这个 id"
        );
        assert_eq!(hub.get(id).unwrap().model(), "model-b", "本会话也换了");

        let err = hub
            .get_mut(id)
            .unwrap()
            .run_command(cmd, "fake:nope")
            .unwrap_err();
        assert!(format!("{err:#}").contains("未知模型"), "{err:#}");
        assert_eq!(written.lock().unwrap().len(), 1, "解析失败就不许落盘");
    }

    /// 装配契约：spec 里的旋钮必须真的到达会话。
    ///
    /// 这条测试是防**静默失效**的：`/compact` 的旋钮曾经写了 getter 却没人
    /// 接上，用户改了 config.yaml 却一切照默认跑——没有任何报错。
    #[test]
    fn the_spec_reaches_the_session_it_specced() {
        let dir = tmp("spec-wiring");
        let mut hub = SessionHub::new(dir.join("sessions.db"));
        let mut sp = spec("http://127.0.0.1:1/v1", "m", "S", &dir);
        sp.compact.retain_tail = 123;
        sp.stream_mode = crate::server::ai::config::StreamMode::Immediate;
        let id = hub.open_new(sp).unwrap();
        let s = hub.get(id).unwrap();
        assert_eq!(s.compact_knobs().retain_tail, 123, "/compact 的旋钮");
        assert_eq!(
            s.stream_mode(),
            crate::server::ai::config::StreamMode::Immediate,
            "交付节拍"
        );
    }

    /// 删会话：**开着但闲着**的能删（用完了就来删，这是常态），
    /// **正在跑一轮**的不给删——回合线程还在往回写，删掉底下的行会让
    /// 它对着空气写。
    #[test]
    fn deleting_a_session_refuses_only_while_a_turn_runs() {
        let dir = tmp("delete-session");
        let mut hub = SessionHub::new(dir.join("sessions.db"));
        let id = hub
            .open_new(spec("http://127.0.0.1:1/v1", "m", "S", &dir))
            .unwrap();
        assert_eq!(hub.list_sessions().unwrap().len(), 1);

        // 开着但闲着：删得掉（内存里也真的没了）。
        assert!(hub.delete_session(id).unwrap());
        assert!(hub.get(id).is_none(), "删完不该还开着");
        assert!(hub.list_sessions().unwrap().is_empty());
        assert!(!hub.delete_session(id).unwrap(), "再删一次是个空操作");

        // 正在跑：拒。
        let busy_id = hub
            .open_new(spec("http://127.0.0.1:1/v1", "m", "S", &dir))
            .unwrap();
        assert!(
            hub.get_mut(busy_id).unwrap().submit("干活"),
            "提交成功（回合已经起来）"
        );
        assert!(hub.get(busy_id).unwrap().busy(), "跑着的时候才算忙");
        let err = hub.delete_session(busy_id).unwrap_err();
        assert!(format!("{err:#}").contains("is running a turn"), "{err:#}");
        assert!(hub.get(busy_id).is_some(), "被拒时不许动状态");
    }

    /// 被拒绝的 `/compact` 必须**说出来**：后台线程里的失败也要落进转录。
    ///
    /// 这条路径和"命令同步拒绝"不同：`run_compact` 起一个后台线程，失败时
    /// 从那边发 `SessionEvent::Error`。静默失败比失败更糟——用户打了一条
    /// 命令，屏幕什么都不动，他无从知道是没压缩还是压缩没成。
    #[test]
    fn a_refused_compaction_still_reaches_the_transcript() {
        use crate::server::entry::Entry;
        let dir = tmp("compact-refusal");
        let mut hub = SessionHub::new(dir.join("sessions.db"));
        let id = hub
            .open_new(spec("http://127.0.0.1:1/v1", "m", "S", &dir))
            .unwrap();
        let cmd = crate::server::commands::lookup("/compact").expect("/compact 在表里");
        hub.get_mut(id).unwrap().run_command(cmd, "").unwrap();

        // 失败产生在后台线程上，所以要轮询着等它上来。
        let mut said = false;
        for _ in 0..200 {
            hub.drain(id);
            said = hub.get(id).unwrap().state.transcript().iter().any(
                |e| matches!(e, Entry::Error { .. }),
            );
            if said {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(said, "后台失败必须落进转录，否则用户看不到任何反馈");
    }

    /// `/compact 2` 里的「2」真的当块数用了：压完的标记落在倒数第 2 个块的
    /// 起点上。参数在会话层解析、切法在后台线程里算——中间任何一环掉了，这里
    /// 看到的都会是默认预算的结果（那样会保下更多块，标记落在更前面）。
    #[test]
    fn compact_block_count_reaches_the_cut() {
        use crate::server::entry::Entry;
        let dir = tmp("compact-blocks");
        let (base, _srv) = fake_gateway(vec![
            sse(&["一答"]),
            sse(&["二答"]),
            sse(&["三答"]),
            // 总结走的是非流式的 `complete`，所以这里要一个 JSON 响应体，
            // 不是 SSE。内容要短过它替换掉的区域（shrink 检查会拒收变大的
            // 总结），四个中文字符的区段配一个字就够。
            chat_json("简"),
        ]);
        let mut hub = SessionHub::new(dir.join("sessions.db"));
        let id = hub.open_new(spec(&base, "m", "S", &dir)).unwrap();
        run_round(&mut hub, id, "一问");
        run_round(&mut hub, id, "二问");
        run_round(&mut hub, id, "三问");

        // 压缩前的转录，用它算「倒数第 2 个块从哪开始」。
        let before = hub.get(id).unwrap().state.transcript().to_vec();
        let blocks = crate::grouping::chunks(&before);
        assert!(blocks.len() > 2, "三轮至少要三块，实际 {}", blocks.len());
        let expected = blocks[blocks.len() - 2].start;

        let cmd = crate::server::commands::lookup("/compact").expect("/compact 在表里");
        hub.get_mut(id).unwrap().run_command(cmd, "2").unwrap();

        let mut seen = None;
        for _ in 0..500 {
            hub.drain(id);
            seen = hub.get(id).unwrap().state.transcript().iter().find_map(|e| {
                match e {
                    Entry::Compaction {
                        first_kept_entry, ..
                    } => Some(*first_kept_entry),
                    _ => None,
                }
            });
            if seen.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(
            seen,
            Some(expected),
            "块数参数没走到 cut（转录里应当有落在倒数第 2 块起点的 Compaction 标记）"
        );
    }

    /// 敲了一个解析不动的参数要**当场报错**，不能把它当焦点、更不能悄悄退回
    /// 默认预算：用户打了数字，就得是那个数字。
    #[test]
    fn an_unparseable_compact_argument_is_refused_at_the_command() {
        let dir = tmp("compact-bad-arg");
        let mut hub = SessionHub::new(dir.join("sessions.db"));
        let id = hub
            .open_new(spec("http://127.0.0.1:1/v1", "m", "S", &dir))
            .unwrap();
        let cmd = crate::server::commands::lookup("/compact").expect("/compact 在表里");
        let e = hub.get_mut(id).unwrap().run_command(cmd, "0").unwrap_err();
        assert!(format!("{e:#}").contains("block count must be at least 1"), "错误要说清下限：{e:#}");
        assert!(!hub.get(id).unwrap().state.busy(), "拒绝不该留下运行中的线程");
    }

    /// 工具名册只在一个会话开始时读一次，**除非**配置允许压缩后再读一次。
    ///
    /// 两个合法缝隙：首 turn 之前，以及压缩之后（对话本来就在重写）。默认
    /// 后者关闭——中途冒出来的工具会让模型手里握着它已经看不见的工具的结果，
    /// 也让 provider 的前缀缓存作废。
    #[test]
    fn the_roster_is_fixed_unless_a_compaction_may_change_it() {
        let dir = tmp("roster-reload");
        let hub_db = dir.join("sessions.db");
        let source: std::sync::Arc<dyn Fn() -> Option<Vec<String>> + Send + Sync> =
            std::sync::Arc::new(|| Some(vec!["bash".into()]));

        let mut with_reload = spec("http://127.0.0.1:1/v1", "m", "S", &dir);
        with_reload.tool_filter = Some(vec!["read".into()]);
        with_reload.roster_source = Some(source.clone());
        with_reload.tools.reload_on_compaction = true;
        let mut hub = SessionHub::new(hub_db.clone());
        let id = hub.open_new(with_reload).unwrap();
        assert_eq!(
            hub.get(id).unwrap().tool_roster().unwrap(),
            &vec!["read".to_string()],
            "开场用的是 profile 里的名册"
        );
        let compaction = || crate::server::events::SessionEvent::Compaction {
            entries: Vec::new(),
            ctx: ChatContext::new(),
            tokens_before: 100,
            tokens_after: 10,
        };
        hub.get_mut(id).unwrap().ingest(compaction());
        assert_eq!(
            hub.get(id).unwrap().tool_roster().unwrap(),
            &vec!["bash".to_string()],
            "允许重读时，压缩后名册该换过来"
        );

        // Same spec, knob off: the roster survives the compaction untouched.
        let mut fixed = spec("http://127.0.0.1:1/v1", "m", "S", &dir);
        fixed.tool_filter = Some(vec!["read".into()]);
        fixed.roster_source = Some(source);
        let id2 = hub.open_new(fixed).unwrap();
        hub.get_mut(id2).unwrap().ingest(compaction());
        assert_eq!(
            hub.get(id2).unwrap().tool_roster().unwrap(),
            &vec!["read".to_string()],
            "关掉之后名册必须不变"
        );
    }

    /// 交付节拍来自 spec：会话建好之后它自己就该知道用哪种。`resume` 走的是
    /// 同一个 `assemble`，所以也吃**当下**的配置（不是上次那份）。
    #[test]
    fn the_stream_mode_rides_the_spec_into_the_session() {
        let dir = tmp("stream-mode");
        let hub_db = dir.join("sessions.db");
        let mut with_buffered = spec("http://127.0.0.1:1/v1", "m", "S", &dir);
        with_buffered.stream_mode = StreamMode::Buffered;
        let id = {
            let mut hub = SessionHub::new(hub_db.clone());
            let id = hub.open_new(with_buffered).unwrap();
            assert_eq!(hub.get(id).unwrap().stream_mode(), StreamMode::Buffered);
            id
        };
        // 新开的 hub（会话不在内存里）走 resume 路径。
        let mut hub = SessionHub::new(hub_db);
        let id = hub
            .resume(id, spec("http://127.0.0.1:1/v1", "m", "S", &dir))
            .unwrap();
        assert_eq!(hub.get(id).unwrap().stream_mode(), StreamMode::Immediate);
    }

    /// 回到历史节点发消息 = **就地分叉**：同一个会话换 tip，内存态不丢。
    ///
    /// 这是这轮修的主 bug 的回归——以前分叉会 `close` 旧会话再 `resume` 新
    /// 会话，而新会话内存态还没开，于是第一条消息被 `hub.submit` 打回来，
    /// daemon 报成"会话 N 正在回复中"。用户看到的就是：回到历史节点发一条
    /// 消息，屏幕说这个会话正忙。
    #[test]
    fn rewinding_with_keeping_forks_a_branch() {
        let dir = tmp("rewind-keep");
        let (base, _srv) = fake_gateway(vec![sse(&["第一答"]), sse(&["分叉答"])]);
        let mut hub = SessionHub::new(dir.join("sessions.db"));
        let a = hub.open_new(spec(&base, "m", "S", &dir)).unwrap();
        run_round(&mut hub, a, "第一问");

        let store_blocks = {
            let store = crate::server::store::Store::open(&hub.db).unwrap();
            store.stored_blocks(a).unwrap()
        };
        let fork_point = store_blocks[0].id;

        // 留：返回新行 id，**旧 id 不再在 hub 里**（同一份内存态换了名）。
        let b = hub.rewind(a, fork_point, false).expect("回退（留）");
        assert_ne!(a, b);
        assert!(hub.get(b).is_some(), "留模式下会话还在内存里");
        assert!(
            hub.get(b).unwrap().session_id() == Some(b),
            "内存里的会话要指向新行"
        );
        // 关键断言：能提交。修之前这里是 false（"正在回复中"）。
        assert!(hub.submit(b, "分叉后的新问题"), "回退后必须能接着发");
        // 分叉后的转录 = 到分叉点为止的前缀。
        assert_eq!(
            hub.get(b).unwrap().transcript().len(),
            2,
            "前缀应当是 用户问 + 模型答"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 剪：同一行内截断，分叉点之后的块**真的从库里没了**，会话接着能用
    /// 且不换行——回退多少次链都不会变长。
    #[test]
    fn rewinding_with_pruning_drops_the_tail() {
        let dir = tmp("rewind-prune");
        let (base, _srv) = fake_gateway(vec![sse(&["第一答"]), sse(&["第二答"])]);
        let mut hub = SessionHub::new(dir.join("sessions.db"));
        let a = hub.open_new(spec(&base, "m", "S", &dir)).unwrap();
        run_round(&mut hub, a, "第一问");
        run_round(&mut hub, a, "第二问");
        let before = crate::server::store::Store::open(&hub.db)
            .unwrap()
            .stored_blocks(a)
            .unwrap();
        assert!(before.len() >= 4, "两轮至少四块，实际 {}", before.len());
        let fork_point = before[0].id;

        let b = hub.rewind(a, fork_point, true).expect("回退（剪）");
        assert_eq!(a, b, "剪枝不换行");
        let after = crate::server::store::Store::open(&hub.db)
            .unwrap()
            .stored_blocks(a)
            .unwrap();
        assert_eq!(after.len(), 1, "分叉点之后的块都该没了");
        assert_eq!(after[0].id, fork_point, "留下的正是回退到的那一块");
        assert_eq!(
            hub.get(b).unwrap().transcript().len(),
            1,
            "前缀 = 第一问，被截掉的那一轮不许留在上下文里"
        );
        assert!(hub.submit(b, "剪枝后的新问题"), "回退后必须能接着发");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 剪枝把回退点之后岔出去的分支整条砍掉——开在内存里的句柄也得跟着走，
    /// 否则它还会往一个已经不存在的会话里写块。
    #[test]
    fn pruning_drops_the_open_handles_of_the_branches_it_takes() {
        let dir = tmp("prune-handles");
        let (base, _srv) = fake_gateway(vec![sse(&["第一答"]), sse(&["第二答"])]);
        let mut hub = SessionHub::new(dir.join("sessions.db"));
        let a = hub.open_new(spec(&base, "m", "S", &dir)).unwrap();
        run_round(&mut hub, a, "第一问");
        run_round(&mut hub, a, "第二问");
        let blocks = crate::server::store::Store::open(&hub.db)
            .unwrap()
            .stored_blocks(a)
            .unwrap();
        let (first, third) = (blocks[0].id, blocks[2].id);

        // 留模式岔一条分支（a 的句柄跟着变成 b），再把 a 自己从库里开回来：
        // 两个会话同时在内存里——这正是"用户切走过、没关"的样子。
        let b = hub.rewind(a, third, false).unwrap();
        assert!(hub.get(b).is_some());
        hub.resume(a, spec(&base, "m", "S", &dir)).unwrap();
        assert!(hub.get(a).is_some(), "两个会话都开着");

        // 回到 a 上剪枝到第一块：b 挂在第三块之后，整条该走。
        assert_eq!(hub.rewind(a, first, true).unwrap(), a, "剪枝不换行");
        assert!(hub.get(a).is_some(), "用户站着的会话必须还在");
        assert!(hub.get(b).is_none(), "被砍掉的分支不该还开着");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 回合**进行中**分叉：旧回合还占着 `busy()`，新分支必须能立刻开工。
    ///
    /// 修之前这里 `submit` 返回 false（"正在回复中"）——分叉只换了 id 和
    /// 转录，没有接管那一轮：`stream.active` 还亮着，前端也就一直显示忙。
    #[test]
    fn forking_mid_round_frees_the_new_branch() {
        let dir = tmp("fork-mid-round");
        let (base, _srv) = fake_gateway(vec![
            sse(&["第一答"]),
            sse(&["第二答"]),
            sse(&["分叉答"]),
        ]);
        let mut hub = SessionHub::new(dir.join("sessions.db"));
        let a = hub.open_new(spec(&base, "m", "S", &dir)).unwrap();
        run_round(&mut hub, a, "第一问");

        let fork_point = {
            let store = crate::server::store::Store::open(&hub.db).unwrap();
            store.stored_blocks(a).unwrap()[0].id
        };

        // 第二轮发出去但**不收事件**：回合还在跑，会话是忙的。
        assert!(hub.submit(a, "第二问"));
        assert!(hub.get(a).unwrap().busy(), "前提：这一轮真的在跑");

        let b = hub.fork_in_place(a, fork_point).expect("就地分叉");
        assert!(!hub.get(b).unwrap().busy(), "分叉后不许再显示忙");
        assert!(hub.submit(b, "分叉后的新问题"), "新分支必须能立刻发消息");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // 跑一轮直到该会话的回合结束（Done）。
    fn run_round(hub: &mut SessionHub, id: i64, text: &str) -> Vec<Change> {
        assert!(hub.submit(id, text), "会话 {id} 应当可以提交");
        let mut all = Vec::new();
        for _ in 0..2000 {
            let changes = hub.drain(id);
            let done = changes.contains(&Change::TurnDone);
            all.extend(changes);
            if done {
                return all;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("会话 {id} 的回合没结束");
    }

    /// 惰性历史：resume 只装尾巴（第一帧才快），但**第一回合之前必须补齐**——
    /// 只把尾巴喂给模型是最难查的一类 bug（模型对着半截对话说话，不报错）。
    /// 网关会把请求体记下来，这里断言最早那条在请求里。
    #[test]
    fn a_resumed_tail_is_completed_before_the_first_turn() {
        let dir = tmp("lazy-history");
        let db = std::env::temp_dir().join(format!(
            "mypi-lazy-history-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::create_dir_all(db.parent().unwrap());
        let _ = std::fs::remove_file(&db);
        let wa = dir.join("proj");
        std::fs::create_dir_all(&wa).unwrap();
        let sid = {
            let mut st = Store::open(&db).unwrap();
            let sid = st
                .create_session("2026-09-26 09:00:00", wa.to_str().unwrap())
                .unwrap();
            let es: Vec<Entry> = (0..500)
                .map(|i| Entry::User {
                    content: format!("m{i}"),
                })
                .collect();
            st.append(sid, &es).unwrap();
            sid
        };

        let (base, srv) = fake_gateway(vec![sse(&["好"])]);
        let mut hub = SessionHub::new(db.clone());
        hub.resume(sid, spec(&base, "model-a", "SYS", &wa)).unwrap();

        let tail = hub.get(sid).unwrap().transcript().len();
        assert!(tail > 0 && tail <= 400, "resume 只该装尾巴，实际 {tail} 条");
        assert!(
            hub.get(sid).unwrap().history_pending(),
            "只装了尾巴就该登记 pending"
        );

        run_round(&mut hub, sid, "继续");
        let seen = srv.join().unwrap();
        assert!(
            seen.iter().any(|r| r.contains("m0")),
            "第一回合的请求里没有最老那条——模型只看到了尾巴"
        );
        assert!(
            !hub.get(sid).unwrap().history_pending(),
            "补齐之后不该还挂着 pending"
        );
    }

    #[test]
    fn two_sessions_in_one_database_never_mix() {
        // 一个 server、两个会话、同一个库文件：数据必须各回各家。
        let dir = tmp("isolate");
        let (base_a, srv_a) = fake_gateway(vec![sse(&["A 的回答"])]);
        let (base_b, srv_b) = fake_gateway(vec![sse(&["B 的回答"])]);
        let mut hub = SessionHub::new(dir.join("sessions.db"));

        let wa = dir.join("proj-a");
        let wb = dir.join("proj-b");
        std::fs::create_dir_all(&wa).unwrap();
        std::fs::create_dir_all(&wb).unwrap();
        let a = hub
            .open_new(spec(&base_a, "model-a", "提示词 A", &wa))
            .unwrap();
        let b = hub
            .open_new(spec(&base_b, "model-b", "提示词 B", &wb))
            .unwrap();
        assert_ne!(a, b, "两个会话是两个 id");
        assert_eq!(hub.len(), 2);

        run_round(&mut hub, a, "A 的问题");
        run_round(&mut hub, b, "B 的问题");
        let _ = srv_a.join();
        let _ = srv_b.join();

        // 转录互不污染
        let ta = hub.get(a).unwrap().transcript().to_vec();
        let tb = hub.get(b).unwrap().transcript().to_vec();
        assert!(
            ta.iter().any(|e| matches!(e, Entry::User { content } if content == "A 的问题"))
        );
        assert!(
            ta.iter().any(|e| matches!(e, Entry::Assistant { content, .. } if content == "A 的回答"))
        );
        assert!(
            !ta.iter().any(|e| matches!(e, Entry::User { content } if content.contains("B"))),
            "A 的转录里不该有 B 的东西：{ta:?}"
        );
        assert!(tb.iter().any(|e| matches!(e, Entry::User { content } if content == "B 的问题")));

        // 库里的请求头也各是各的：模型、系统提示词、cwd 都不串
        let store = Store::open(&hub.db).unwrap();
        let ra = store.rounds(a).unwrap();
        let rb = store.rounds(b).unwrap();
        assert_eq!(ra.len(), 1);
        assert_eq!(rb.len(), 1);
        assert_eq!(ra[0].model, "model-a");
        assert_eq!(ra[0].system, "提示词 A");
        assert_eq!(rb[0].model, "model-b");
        assert_eq!(rb[0].system, "提示词 B");
        assert_eq!(store.session(a).unwrap().cwd.as_deref(), wa.to_str());
        assert_eq!(store.session(b).unwrap().cwd.as_deref(), wb.to_str());

        // 各自都能从库里复现自己的请求
        let rp_a = hub.get(a).unwrap().replay_round(a, 1).unwrap();
        let rp_b = hub.get(b).unwrap().replay_round(b, 1).unwrap();
        assert_eq!(rp_a.body["model"], serde_json::json!("model-a"));
        assert_eq!(rp_b.body["model"], serde_json::json!("model-b"));
        assert_eq!(
            rp_a.messages.as_array().unwrap().len(),
            3,
            "system + 问 + 答"
        );

        // 关掉 A 不影响 B
        let _ = hub.close(a);
        assert_eq!(hub.ids(), vec![b]);
        assert!(hub.get(b).unwrap().transcript().len() >= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn two_sessions_can_be_mid_generation_at_once() {
        // 真并发：两个会话各自一个回合线程同时跑，同一个库文件同时写。
        // 各自的 Change 只能回到自己的 id。
        let dir = tmp("concurrent");
        let (base_a, srv_a) = fake_gateway(vec![sse(&["甲", "回"])]);
        let (base_b, srv_b) = fake_gateway(vec![sse(&["乙", "回"])]);
        let mut hub = SessionHub::new(dir.join("sessions.db"));
        let a = hub
            .open_new(spec(&base_a, "model-a", "S", &dir.join("a")))
            .unwrap();
        let b = hub
            .open_new(spec(&base_b, "model-b", "S", &dir.join("b")))
            .unwrap();

        // 两个回合同时在飞
        assert!(hub.submit(a, "问甲"));
        assert!(hub.submit(b, "问乙"));
        assert!(!hub.submit(a, "再来一个"), "同一会话不许并发第二轮");

        let mut done = std::collections::BTreeSet::new();
        let mut changes_a = Vec::new();
        let mut changes_b = Vec::new();
        for _ in 0..2000 {
            for (id, changes) in hub.drain_all() {
                if changes.contains(&Change::TurnDone) {
                    done.insert(id);
                }
                if id == a {
                    changes_a.extend(changes);
                } else if id == b {
                    changes_b.extend(changes);
                } else {
                    panic!("出现了第三个会话 {id}");
                }
            }
            if done.len() == 2 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(done.len(), 2, "两个会话都必须完成");
        let _ = srv_a.join();
        let _ = srv_b.join();
        assert!(changes_a.contains(&Change::Stream));
        assert!(changes_b.contains(&Change::Stream));

        // 两边都落了盘，而且互相没碰过
        let store = Store::open(&hub.db).unwrap();
        assert_eq!(store.load_entries(a).unwrap().len(), 2);
        assert_eq!(store.load_entries(b).unwrap().len(), 2);
        let text_a = match &store.load_entries(a).unwrap()[1] {
            Entry::Assistant { content, .. } => content.clone(),
            other => panic!("{other:?}"),
        };
        let text_b = match &store.load_entries(b).unwrap()[1] {
            Entry::Assistant { content, .. } => content.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(text_a, "甲回");
        assert_eq!(text_b, "乙回");
        assert_eq!(hub.status(a), Some(RunState::Idle));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resume_brings_back_the_stored_conversation_and_where_it_ran() {
        let dir = tmp("resume");
        let (base, srv) = fake_gateway(vec![sse(&["第一答"])]);
        let mut hub = SessionHub::new(dir.join("sessions.db"));
        let work = dir.join("work");
        std::fs::create_dir_all(&work).unwrap();
        let id = hub
            .open_new(spec(&base, "model-a", "老提示词", &work))
            .unwrap();
        run_round(&mut hub, id, "第一问");
        let _ = srv.join();
        let _ = hub.close(id);

        // 换个「前端」重开：新的 client、新的提示词、新的 cwd —— 但转录和 cwd 来自库。
        let elsewhere = dir.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        let (base2, srv2) = fake_gateway(vec![sse(&["第二答"])]);
        assert_eq!(hub.resume(id, spec(&base2, "model-b", "新提示词", &elsewhere)).unwrap(), id);
        assert_eq!(hub.get(id).unwrap().cwd(), work, "回到它当时所在的目录");
        let tx = hub.get(id).unwrap().transcript().to_vec();
        assert!(
            tx.iter().any(|e| matches!(e, Entry::User { content } if content == "第一问")),
            "旧的转录要回来：{tx:?}"
        );
        assert!(tx.iter().any(|e| matches!(e, Entry::Assistant { content, .. } if content == "第一答")));

        // 第二轮用新提示词，但上下文里带着旧对话
        run_round(&mut hub, id, "第二问");
        let _ = srv2.join();
        let store = Store::open(&hub.db).unwrap();
        let rows = store.rounds(id).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].system, "老提示词");
        assert_eq!(rows[1].system, "新提示词");
        assert_eq!(rows[1].model, "model-b");
        let entries = store.load_entries(id).unwrap();
        assert_eq!(entries.len(), 4, "一问一答 ×2");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unknown_session_is_not_a_panic() {
        let dir = tmp("unknown");
        let mut hub = SessionHub::new(dir.join("sessions.db"));
        assert_eq!(hub.status(42), None);
        assert!(!hub.submit(42, "喂"));
        assert!(hub.drain(42).is_empty());
        assert!(!hub.close(42));
        assert!(hub.get(42).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
