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
//!                            │  sessions.db (WAL: readers never block, writers queue)
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
            .ok_or_else(|| anyhow::anyhow!("无法创建会话（数据库不可用）"))?;
        self.sessions.insert(id, OpenSession { session, rx });
        Ok(id)
    }

    /// Reopen a stored session: load its transcript and cwd, rebuild the model
    /// context from the entries (the same reconstruction a live run uses), and
    /// adopt the given client/system prompt for **future** rounds.
    ///
    /// The stored working directory wins over `spec.cwd`: a resumed session
    /// belongs where it was, not where the process happens to be.
    pub fn resume(&mut self, id: i64, spec: SessionSpec) -> anyhow::Result<i64> {
        if self.sessions.contains_key(&id) {
            return Ok(id);
        }
        let store = Store::open(&self.db)?;
        let meta = store.session(id)?;
        let entries = store.load_entries(id)?;
        // Name and cwd come from storage, not from the caller: a resumed
        // session belongs where it was, under the name it had.
        let name = store.effective_name(id)?;
        let cwd_seq = store
            .cwd_history(id)?
            .last()
            .map(|(seq, _)| *seq)
            .unwrap_or(0);
        let cwd = meta
            .cwd
            .as_deref()
            .map(PathBuf::from)
            .unwrap_or_else(|| spec.cwd.clone());
        // The model context is rebuilt from the entries by the same function a
        // live turn uses, so a resumed session sees the conversation it had.
        let chat = crate::server::turn::entries_to_context(&spec.system_prompt, &entries);
        let (mut session, rx) = self.assemble(SessionState::new(Some(store)), spec, chat);
        let _ = session.adopt_session(id, entries, name);
        session.set_cwd(cwd);
        session.set_cwd_seq(cwd_seq);
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
            anyhow::bail!("会话 {id} 正在跑一轮，先打断再删");
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::ai::client::Client;
    use crate::server::entry::Entry;
    use crate::server::test_gateway::{fake_gateway, sse};

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
            context_window: 0,
            model_name: model.to_string(),
            cwd: cwd.to_path_buf(),
            tool_filter: None,
            roster_source: None,
            tools: ToolsConfig::default(),
            compact: Default::default(),
            commands: Default::default(),
            browser: BrowserConfig::default(),
            stream_mode: StreamMode::default(),
        }
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
                        name: "FAKE-B".into(),
                        context_window: 999,
                        max_tokens: 1234,
                        cost: Cost::default(),
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
        assert!(format!("{err:#}").contains("没接"), "{err:#}");
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
                    Ok(("你现在是审阅者".to_string(), Some(vec!["read".into(), "grep".into()])))
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
                        name: "FAKE-B".into(),
                        context_window: 42,
                        max_tokens: 1,
                        cost: Cost::default(),
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
        assert!(format!("{err:#}").contains("正在跑"), "{err:#}");
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
