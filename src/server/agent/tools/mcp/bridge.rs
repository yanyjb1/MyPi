//! The MCP backend seam: no rmcp or stdio types cross this boundary.

use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

use crate::platform::stop::Stop;

#[derive(Debug, Clone)]
pub struct ServerInfo {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub required: Vec<String>,
    /// The tool's declared parameter map, verbatim from its `inputSchema`.
    pub parameters: Value,
}

#[derive(Debug, Clone)]
pub struct McpResult {
    pub content: Vec<McpContent>,
    pub is_error: bool,
}

#[derive(Debug, Clone)]
pub enum McpContent {
    Text(String),
    Unsupported { kind: String },
}

pub trait McpBridge: Send + Sync {
    fn servers(&self) -> Vec<ServerInfo>;
    fn tools(&self, server: &str) -> Result<Vec<ToolInfo>>;
    /// One tool call. A stop that lands meanwhile kills the server; the next call starts it again.
    fn call(&self, server: &str, tool: &str, args: Value, stop: &Arc<Stop>) -> Result<McpResult>;
    /// Re-read the table for where the session stands; a no-op when the table is unchanged.
    fn rescan(&self, cwd: &Path) -> Result<()>;
    fn shutdown(&self);
}
