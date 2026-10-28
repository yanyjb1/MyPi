//! Tools. The root holds the four shared pieces: `spec`, `registry`, `executor`, `workspace`; tools
//! are grouped by use domain.

mod bash;
mod executor;
pub(crate) mod jobs;
mod kairos;
mod mcp;
mod memo;
mod registry;
mod spec;
pub(crate) mod subagent;
mod workspace;

#[cfg(feature = "web")]
mod web;
#[cfg(feature = "web")]
pub(crate) use web::close_session as close_web_session;

pub use bash::context::Window;
pub use executor::execute;
pub use mcp::{McpBridge, McpContent, McpResult, ServerInfo, SyncMcpBridge, ToolInfo};
pub use memo::todo::state::{TodoPhase, TodoStatus, TodoTask};
pub use registry::{TOOLS, roster};
pub use spec::{
    Args, Artifacts, BashEnv, Env, Inbox, Kind, MemoRead, MemoReadOp, MemoVersion, MemoView, Param,
    Run, Spill, Tool, ToolOutput, ToolProgress,
};
pub use workspace::Workspace;
