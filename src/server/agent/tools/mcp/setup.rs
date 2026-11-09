//! The server table: `mcp.json`, the project's copy replacing the global one.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Deserialize;

/// One configured server, as `mcp.json` writes it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ServerConfig {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_timeout() -> u64 {
    120
}

#[derive(Debug, Deserialize)]
struct File {
    #[serde(default)]
    servers: BTreeMap<String, ServerConfig>,
}

/// The table for where the session stands: the project's copy when it exists, else the global one.
pub fn table(cwd: &Path, global: &Path) -> Result<BTreeMap<String, ServerConfig>> {
    let project = cwd
        .join(crate::server::config::paths::PROJECT_DIR)
        .join("mcp.json");
    let path = if project.is_file() {
        project
    } else if global.is_file() {
        global.to_path_buf()
    } else {
        return Ok(BTreeMap::new());
    };
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let file: File = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(file.servers)
}
