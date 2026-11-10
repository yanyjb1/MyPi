//! The ports the loop implements: what a tool may read or hand over, never a database handle.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

use super::job;
use crate::server::agent::tools::mcp::bridge::McpBridge;

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
    /// The newest result block whose memo carries `marker`, with that memo: a replay's anchor.
    fn last_memo(&self, session: i64, marker: &str) -> Result<Option<(i64, String)>>;
    fn block(&self, session: i64, id: i64) -> Result<Option<BlockPair>>;
    fn jobs(&self, session: i64) -> Option<Arc<dyn job::JobHost>>;
    fn child(&self, session: i64) -> Option<Arc<dyn Child>>;
    /// The session's own MCP bridge, opened where that session now stands.
    fn mcp(&self, session: i64, cwd: &Path) -> Result<Arc<dyn McpBridge>>;
}

/// One sub-session's live state: its draft, its budget and its answer file.
pub trait Child: Send + Sync {
    fn draft(&self) -> &dyn Draft;
    /// What is left of the tool-call budget.
    fn left(&self) -> u32;
    /// Spend one call; the caller refuses first, so this never goes below zero.
    fn charge(&self);
    /// The file `final submit` writes; its name was fixed when the child was spawned.
    fn answer(&self) -> &Path;
    /// Record that the draft has landed; the parent's job is done only after this.
    fn submit(&self);
    fn submitted(&self) -> bool;
    /// The nudge to append when a round ends without a submit; `None` once they are spent.
    fn nag(&self) -> Option<&'static str>;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Before,
    After,
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
