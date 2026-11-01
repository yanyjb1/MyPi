//! The tool layer's outward shape: one batch in, one result per call out.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

use super::jobs::JobService;
use super::mcp::McpBridge;
use super::workspace::Workspace;
use crate::server::agent::request::contract::{Spill, ToolCall};
use crate::server::agent::request::live::Event;

/// The session's working directory handle: `cd` writes it, the next batch reads it.
pub type Cwd = Arc<RwLock<PathBuf>>;

/// One batch's input.
pub struct BatchInput {
    pub session: i64,
    pub turn: i64,
    /// The session's start directory; it never moves.
    pub root: PathBuf,
    pub cwd: Cwd,
    /// The session's tool names; the one list the profile gate reads.
    pub tools: Vec<String>,
    /// This batch, in execution order.
    pub calls: Vec<ToolCall>,
    pub stop: Arc<AtomicBool>,
    pub out: Sender<Event>,
}

/// The tool layer, as the loop sees it.
pub trait Batch: Send + Sync {
    fn run(&self, input: BatchInput) -> Result<Vec<ToolResult>>;
}

/// One finished call.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub call_id: String,
    pub ok: bool,
    pub result: String,
    pub memo: Option<String>,
    pub spill: Option<Spill>,
    pub duration_ms: u64,
    pub effects: Vec<Effect>,
    pub stop: bool,
}

/// One side effect the loop turns into rows, anchored on the result block.
#[derive(Debug, Clone)]
pub enum Effect {
    Memo {
        op: MemoOp,
        name: String,
        content: Option<String>,
        ts: String,
    },
    Alias {
        op: AliasOp,
        name: String,
        command: Option<String>,
        ts: String,
    },
    Child {
        tools: Vec<String>,
        cwd: PathBuf,
        start: String,
        max_tool_calls: u32,
    },
}

impl Effect {
    /// Fill the timestamp the front door stamps for every effect.
    pub fn stamp(&mut self, ts: &str) {
        match self {
            Effect::Memo { ts: at, .. } | Effect::Alias { ts: at, .. } => *at = ts.to_string(),
            Effect::Child { .. } => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoOp {
    Set,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasOp {
    Add,
    Remove,
}

/// A tool's return, before the front door turns it into a result.
#[derive(Debug)]
pub struct ToolOutput {
    pub text: String,
    pub memo: Option<Value>,
    pub effects: Vec<Effect>,
    pub stop: bool,
}

impl ToolOutput {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            memo: None,
            effects: Vec::new(),
            stop: false,
        }
    }

    pub fn with_memo(text: impl Into<String>, memo: Value) -> Self {
        Self {
            text: text.into(),
            memo: Some(memo),
            effects: Vec::new(),
            stop: false,
        }
    }

    pub fn effect(mut self, effect: Effect) -> Self {
        self.effects.push(effect);
        self
    }

    pub fn stop(mut self) -> Self {
        self.stop = true;
        self
    }
}

/// What a tool reads: one session's conditions for one batch.
pub struct Env {
    /// The start directory and the working directory this batch sees.
    pub workspace: Workspace,
    pub limits: Limits,
    pub bash: BashEnv,
    pub browser: Browser,
    pub session: i64,
    pub batch_len: usize,
    pub tools: BTreeSet<String>,
    pub aliases: BTreeMap<String, String>,
    /// The budget a spawned sub-session gets when the call names none.
    pub default_budget: u32,
    pub mcp: Option<Arc<dyn McpBridge>>,
    pub export: Arc<dyn Export>,
    pub paths: Paths,
    pub source: Arc<dyn Source>,
}

impl Env {
    /// Is this tool in the session's list? The profile gate reads the same list.
    pub fn allows(&self, name: &str) -> bool {
        self.tools.contains(name)
    }
}

/// Inline budget for one tool result.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub lines: usize,
    pub bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            lines: 512,
            bytes: 262_144,
        }
    }
}

/// What `bash` runs with.
pub struct BashEnv {
    pub timeout: Duration,
    pub interrupt: Arc<AtomicBool>,
    pub shell: crate::server::platform::shell::Shell,
}

/// What the web tool launches with.
#[derive(Debug, Clone)]
pub struct Browser {
    pub bin: Option<PathBuf>,
    pub port: Option<u16>,
    pub profile_dir: Option<PathBuf>,
    pub headless: bool,
    pub user_agent: Option<String>,
    pub extra_args: Vec<String>,
}

impl Default for Browser {
    fn default() -> Self {
        Self {
            bin: None,
            port: None,
            profile_dir: None,
            headless: true,
            user_agent: None,
            extra_args: Vec::new(),
        }
    }
}

/// Where the project sits and where its two folders are.
#[derive(Debug, Clone)]
pub struct Paths {
    pub project: PathBuf,
    pub agents: PathBuf,
    pub answers: PathBuf,
}

/// Reads the loop owns; tools never open the database.
pub trait Source: Send + Sync {
    fn memo(&self, session: i64) -> Arc<dyn MemoRead>;
    fn aliases(&self, session: i64) -> Result<BTreeMap<String, String>>;
    fn history(
        &self,
        session: i64,
        anchor: &str,
        direction: Direction,
        limit: usize,
        tool: Option<&str>,
    ) -> Result<Vec<CallRecord>>;
    fn calls_after(&self, session: i64, block: i64, tool: &str) -> Result<Vec<CallRecord>>;
    /// The newest result block whose memo carries `marker`, with that memo: where a replay restarts.
    fn last_reset(&self, session: i64, marker: &str) -> Result<Option<(i64, String)>>;
    fn block(&self, session: i64, id: i64) -> Result<Option<BlockPair>>;
    fn jobs(&self, session: i64) -> Option<Arc<JobService>>;
    fn child(&self, session: i64) -> Option<Arc<dyn Child>>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Back,
    Forward,
}

/// One call, as history queries hand it over.
#[derive(Debug, Clone)]
pub struct CallRecord {
    /// The request block's id: what `context show` and an anchor take as a number.
    pub block: i64,
    pub tool: String,
    pub args: String,
    pub result: Option<String>,
    pub ok: bool,
}

/// A call's two rows, whole, for `context show`.
#[derive(Debug, Clone)]
pub struct BlockPair {
    pub request: Value,
    pub result: Option<Value>,
}

/// One sub-session's live state: its draft and what is left of its budget.
pub trait Child: Send + Sync {
    fn is_child(&self) -> bool;
    fn draft(&self) -> &dyn Draft;
    fn left(&self) -> Option<u32>;
    fn charge(&self) -> Result<()>;
}

/// The draft a sub-session leaves behind. The tool applies its own edit semantics and writes back.
pub trait Draft: Send + Sync {
    fn read(&self) -> Result<String>;
    fn write(&self, text: &str) -> Result<()>;
}

/// Full texts live in the database; this hands one out as a file on demand.
pub trait Export: Send + Sync {
    fn dir(&self) -> &Path;
    fn path(&self, reference: &str) -> Result<PathBuf>;
}

/// What the assembly offers that is the same for every session.
pub trait Furnish: Send + Sync {
    fn mcp(&self) -> Option<Arc<dyn McpBridge>>;
}

/// Memo reads for the `memo` tool; implemented by the caller, never by the tool.
pub trait MemoRead: Send + Sync {
    fn read(
        &self,
        session: i64,
        scope: MemoScope,
        op: MemoReadOp,
        name: Option<&str>,
    ) -> Result<MemoView>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoScope {
    Current,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoReadOp {
    List,
    Show,
    History,
}

/// One stored memo version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoVersion {
    pub id: i64,
    pub content: String,
    pub ts: String,
}

/// What a memo read answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MemoView {
    Names(Vec<String>),
    Content(String),
    History(Vec<MemoVersion>),
}
