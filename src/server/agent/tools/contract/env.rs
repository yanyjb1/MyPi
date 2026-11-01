//! One session's conditions for one batch: budgets, shell, folders, and the reads it holds.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use serde_json::Value;

use super::view::{Export, Furnish, Source};
use crate::server::agent::tools::mcp::McpBridge;
use crate::server::agent::tools::workspace::Workspace;

/// What a tool reads: one session's conditions for one batch.
pub struct Env {
    /// The start directory and the working directory this batch sees.
    pub workspace: Workspace,
    pub limits: Limits,
    pub bash: BashEnv,
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

/// Where the project sits and where its folders are.
#[derive(Debug, Clone)]
pub struct Paths {
    pub project: PathBuf,
    /// The project's templates.
    pub agents: PathBuf,
    /// The answers a sub-session's final text lands in.
    pub answers: PathBuf,
    /// The templates that belong to the user, not to this project.
    pub global_agents: PathBuf,
    /// The project's MCP server table; when it exists it replaces the global one.
    pub mcp: PathBuf,
    /// The user-wide MCP server table, read only when the project has none.
    pub global_mcp: PathBuf,
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
