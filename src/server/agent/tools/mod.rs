//! Tools. The root holds the shared pieces: `contract`, `engine`, `spec`, `registry`, `workspace`;
//! tools are grouped by use domain.

mod bash;
pub mod contract;
pub mod engine;
mod kairos;
mod mcp;
mod memo;
mod registry;
mod spec;
pub(crate) mod subagent;
mod summary;
mod workspace;

#[cfg(feature = "web")]
mod web;
#[cfg(feature = "web")]
pub(crate) use web::close_session as close_web_session;

pub use contract::{
    AliasOp, BashEnv, Batch, BatchInput, BlockPair, Browser, CallRecord, Child, Cwd, Direction,
    Draft, Effect, Env, Export, Furnish, Limits, MemoOp, MemoRead, MemoReadOp, MemoScope,
    MemoVersion, MemoView, Paths, Source, ToolOutput, ToolResult,
};
pub use engine::Engine;
pub use mcp::{McpBridge, McpContent, McpResult, ServerInfo, SyncMcpBridge, ToolInfo};
pub(crate) use memo::todo::current as todo_current;
pub use memo::todo::state::TodoItem;
pub use registry::{Availability, TOOLS, roster};
pub use spec::{Args, Kind, Param, Run, Summary, Tool, ToolProgress};
pub use workspace::Workspace;
