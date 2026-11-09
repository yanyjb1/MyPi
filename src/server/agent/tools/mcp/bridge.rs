//! The MCP backend seam: no rmcp or stdio types cross this boundary.

use std::path::Path;

use anyhow::Result;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct ServerInfo {
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub required: Vec<String>,
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
    fn call(&self, server: &str, tool: &str, args: Value) -> Result<McpResult>;
    /// Re-read the table for where the session stands; a no-op when the table is unchanged.
    fn rescan(&self, cwd: &Path) -> Result<()>;
    fn shutdown(&self);
}
