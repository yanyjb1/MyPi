//! The session's own state: what this front end knows about the session it is on.
//! Design: `README.md`.

use crate::server::agent::tools::TodoPhase;
use crate::server::api::call::meta::SessionState;
use crate::server::api::call::sessions::SessionRow;
use crate::server::api::call::usage::ModelUsage;
use crate::server::api::types::{CandidatePools, UsageLang};
use crate::server::storage::SessionMeta;

/// How bad a notice is. It picks the color, nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Info,
    Error,
}

/// How long a notice lives: short = until the next persistent thing, long = until the reader
/// leaves the conversation. Design: `README.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Life {
    Short,
    Long,
}

/// Something this front end has to say. It is never a block and never reaches the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    pub text: String,
    pub level: Level,
    pub life: Life,
}

/// The usage bill for one session, in the language the panel speaks.
pub struct UsagePanel {
    pub lang: UsageLang,
    pub groups: Vec<ModelUsage>,
}

/// What this front end knows about the session it is on.
pub struct State {
    /// The session it is on; `None` = a draft (no row exists yet).
    attached: Option<i64>,
    meta: Option<SessionMeta>,
    /// Where the session works right now. The model's `cd` moves it, nothing writes it.
    cwd: String,
    busy: bool,
    usage: Option<UsagePanel>,
    /// The picker's rows, as last asked for; `None` = the question has not been answered yet.
    rows: Option<Vec<SessionRow>>,
    pools: Option<CandidatePools>,
    /// The task list, replayed by the server from the session's own tool calls.
    todo: Vec<TodoPhase>,
    notices: Vec<Notice>,
    /// The last write was refused because the session is mid-round.
    refused: bool,
    /// The model's context window, once a front end has asked the server for it.
    ctx_limit: Option<u64>,
    /// A row was created by this front end and has not been opened yet.
    new_born: bool,
}

impl State {
    pub fn new(cwd: String) -> State {
        State {
            attached: None,
            meta: None,
            cwd,
            busy: false,
            usage: None,
            rows: None,
            pools: None,
            todo: Vec::new(),
            notices: Vec::new(),
            refused: false,
            ctx_limit: None,
            new_born: false,
        }
    }

    pub fn attached(&self) -> Option<i64> {
        self.attached
    }

    /// Enter a session: the row and the list are about to arrive, the notices are gone.
    pub fn attach(&mut self, id: i64) {
        self.attached = Some(id);
        self.meta = None;
        self.todo.clear();
        self.notices.clear();
        self.refused = false;
        self.ctx_limit = None;
        self.usage = None;
        self.busy = false;
    }

    /// Leave the session: everything about it goes, the draft state is empty.
    pub fn detach(&mut self) {
        self.attached = None;
        self.meta = None;
        self.todo.clear();
        self.notices.clear();
        self.busy = false;
        self.usage = None;
        self.refused = false;
        self.ctx_limit = None;
        self.new_born = false;
    }

    pub fn set_meta(&mut self, state: SessionState) {
        self.meta = Some(state.meta);
        self.cwd = state.cwd.to_string_lossy().into_owned();
    }

    pub fn name(&self) -> Option<&str> {
        self.meta.as_ref().and_then(|m| m.name.as_deref())
    }

    /// The model this session is on; `None` = it follows the configured default.
    pub fn model(&self) -> Option<&str> {
        self.meta.as_ref().and_then(|m| m.model.as_deref())
    }

    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    /// Did the model move the working directory away from the row's own value?
    pub fn cwd_temporary(&self) -> bool {
        match self.meta.as_ref().and_then(|m| m.cwd.as_deref()) {
            Some(row) => row != self.cwd,
            None => false,
        }
    }

    pub fn set_cwd(&mut self, cwd: String) {
        self.cwd = cwd;
    }

    /// `/name` — landed locally, before the server confirms it.
    pub fn set_name(&mut self, name: Option<String>) {
        if let Some(meta) = &mut self.meta {
            meta.name = name;
        }
    }

    /// `/switch` — this session's model, landed locally.
    pub fn set_model(&mut self, provider: String, model: String) {
        if let Some(meta) = &mut self.meta {
            meta.model_provider = Some(provider);
            meta.model = Some(model);
        }
    }

    /// `/cdp` — the working directory, resolved against the live one. The server's own answer
    /// corrects the guess.
    pub fn migrate(&mut self, path: &str) {
        let target = std::path::Path::new(&self.cwd)
            .join(path)
            .to_string_lossy()
            .into_owned();
        self.cwd = target.clone();
        if let Some(meta) = &mut self.meta {
            meta.cwd = Some(target);
        }
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    pub fn set_busy(&mut self, busy: bool) {
        self.busy = busy;
    }

    pub fn set_usage(&mut self, lang: UsageLang, groups: Vec<ModelUsage>) {
        self.usage = Some(UsagePanel { lang, groups });
    }

    pub fn usage(&self) -> Option<&UsagePanel> {
        self.usage.as_ref()
    }

    pub fn set_rows(&mut self, rows: Vec<SessionRow>) {
        self.rows = Some(rows);
    }

    pub fn rows(&self) -> Option<&[SessionRow]> {
        self.rows.as_deref()
    }

    pub fn set_pools(&mut self, pools: CandidatePools) {
        self.pools = Some(pools);
    }

    pub fn pools(&self) -> Option<&CandidatePools> {
        self.pools.as_ref()
    }

    pub fn set_todo(&mut self, todo: Vec<TodoPhase>) {
        self.todo = todo;
    }

    pub fn todo(&self) -> &[TodoPhase] {
        &self.todo
    }

    pub fn notices(&self) -> &[Notice] {
        &self.notices
    }

    /// The model's context window, or `None` when nobody has asked yet.
    pub fn ctx_limit(&self) -> Option<u64> {
        self.ctx_limit
    }

    pub fn set_ctx_limit(&mut self, limit: u64) {
        self.ctx_limit = Some(limit);
    }

    /// Take the new-row flag; `true` means this front end just created the row.
    pub fn take_new_born(&mut self) -> bool {
        std::mem::take(&mut self.new_born)
    }

    pub fn set_new_born(&mut self, value: bool) {
        self.new_born = value;
    }

    /// Whether the last write was refused (the session is mid-round).
    pub fn refused(&self) -> bool {
        self.refused
    }

    pub fn set_refused(&mut self, refused: bool) {
        self.refused = refused;
    }

    /// Say something. A long notice clears the short ones, the way a block does.
    pub fn notice(&mut self, text: String, level: Level, life: Life) {
        if life == Life::Long {
            self.settle();
        }
        self.notices.push(Notice { text, level, life });
    }

    /// A persistent thing arrived (a block landed): the short-lived notices go.
    pub fn settle(&mut self) {
        self.notices.retain(|n| n.life == Life::Long);
    }
}
