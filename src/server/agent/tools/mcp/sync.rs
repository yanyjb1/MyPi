//! The default MCP backend: one lazy stdio server per configured name, keyed to where the
//! session stands.
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use super::bridge::{McpBridge, McpContent, McpResult, ServerInfo, ToolInfo};
use super::setup::ServerConfig;
use super::stdio::{self, Server};

pub struct SyncMcpBridge {
    standing: Mutex<Standing>,
    global: std::path::PathBuf,
    servers: Mutex<HashMap<String, Arc<Mutex<Server>>>>,
}

/// Where the session stands and the table read for that place; the two change together.
struct Standing {
    cwd: std::path::PathBuf,
    config: BTreeMap<String, ServerConfig>,
}

impl SyncMcpBridge {
    pub fn new(
        cwd: std::path::PathBuf,
        global: std::path::PathBuf,
        config: BTreeMap<String, ServerConfig>,
    ) -> Self {
        Self {
            standing: Mutex::new(Standing { cwd, config }),
            global,
            servers: Mutex::new(HashMap::new()),
        }
    }

    fn server_for(&self, name: &str) -> Result<Arc<Mutex<Server>>> {
        let existing = self
            .servers
            .lock()
            .map_err(|_| anyhow!("MCP server table is poisoned"))?
            .get(name)
            .cloned();
        if let Some(server) = existing {
            let closed = {
                let server = server
                    .lock()
                    .map_err(|_| anyhow!("MCP server `{name}` is poisoned"))?;
                server.is_closed()
            };
            if !closed {
                return Ok(server);
            }
            self.servers
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(name);
        }
        let (mut config, cwd) = {
            let standing = self
                .standing
                .lock()
                .map_err(|_| anyhow!("MCP server table is poisoned"))?;
            let config = standing
                .config
                .get(name)
                .cloned()
                .ok_or_else(|| anyhow!("unknown MCP server `{name}`"))?;
            (config, standing.cwd.clone())
        };
        // Relative paths anchor where the session stands, not where the process launched.
        config.cwd = Some(cwd);
        let mut server = stdio::spawn(name, &config)?;
        server
            .initialize()
            .map_err(|e| anyhow!("MCP server `{name}` handshake failed: {e:#}"))?;
        let server = Arc::new(Mutex::new(server));
        self.servers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(name.to_string(), server.clone());
        Ok(server)
    }

    fn tools_of(&self, name: &str) -> Result<Vec<ToolInfo>> {
        let server = self.server_for(name)?;
        let mut cursor: Option<String> = None;
        let mut out = Vec::new();
        loop {
            let params = cursor
                .as_ref()
                .map(|cursor| json!({"cursor": cursor}))
                .unwrap_or_else(|| json!({}));
            let value = {
                let mut server = server
                    .lock()
                    .map_err(|_| anyhow!("MCP server `{name}` is poisoned"))?;
                server.request("tools/list", params)?
            };
            let object = value
                .as_object()
                .ok_or_else(|| anyhow!("MCP server `{name}` returned a non-object tools/list"))?;
            let tools = object
                .get("tools")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("MCP server `{name}` returned no tools array"))?;
            for tool in tools {
                out.push(tool_of(name, tool)?);
            }
            let next = object
                .get("nextCursor")
                .and_then(Value::as_str)
                .filter(|cursor| !cursor.is_empty())
                .map(str::to_string);
            if next.is_none() || next == cursor {
                break;
            }
            cursor = next;
        }
        Ok(out)
    }

    fn call_tool(&self, name: &str, tool: &str, args: Value) -> Result<McpResult> {
        let server = self.server_for(name)?;
        let value = {
            let mut server = server
                .lock()
                .map_err(|_| anyhow!("MCP server `{name}` is poisoned"))?;
            server.request("tools/call", json!({"name": tool, "arguments": args}))?
        };
        result_of(name, value)
    }
}

fn tool_of(server: &str, value: &Value) -> Result<ToolInfo> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("MCP server `{server}` returned a non-object tool"))?;
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("MCP server `{server}` returned a tool without a name"))?;
    let description = object
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let required = object
        .get("inputSchema")
        .and_then(Value::as_object)
        .and_then(|schema| schema.get("required"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Ok(ToolInfo {
        name: name.to_string(),
        description,
        required,
    })
}

fn result_of(server: &str, value: Value) -> Result<McpResult> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("MCP server `{server}` returned a non-object call result"))?;
    let is_error = object
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let content = match object.get("content") {
        None => Vec::new(),
        Some(value) => {
            let items = value
                .as_array()
                .ok_or_else(|| anyhow!("MCP server `{server}` returned a non-array content"))?;
            items
                .iter()
                .map(|item| content_of(server, item))
                .collect::<Result<Vec<_>>>()?
        }
    };
    Ok(McpResult { content, is_error })
}

fn content_of(server: &str, value: &Value) -> Result<McpContent> {
    let object = value
        .as_object()
        .ok_or_else(|| anyhow!("MCP server `{server}` returned a non-object content item"))?;
    let kind = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if kind == "text" {
        return Ok(McpContent::Text(
            object
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ));
    }
    Ok(McpContent::Unsupported {
        kind: kind.to_string(),
    })
}

impl McpBridge for SyncMcpBridge {
    fn rescan(&self, cwd: &Path) -> Result<()> {
        let table = super::setup::table(cwd, &self.global)?;
        {
            let mut standing = self
                .standing
                .lock()
                .map_err(|_| anyhow!("MCP server table is poisoned"))?;
            if standing.cwd.as_path() == cwd && standing.config == table {
                return Ok(());
            }
            standing.cwd = cwd.to_path_buf();
            standing.config = table;
        }
        let mut servers = self.servers.lock().unwrap_or_else(|e| e.into_inner());
        for (_, server) in servers.drain() {
            let mut server = server.lock().unwrap_or_else(|e| e.into_inner());
            server.shutdown();
        }
        Ok(())
    }

    fn servers(&self) -> Vec<ServerInfo> {
        self.standing
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .config
            .keys()
            .map(|name| ServerInfo { name: name.clone() })
            .collect()
    }

    fn tools(&self, server: &str) -> Result<Vec<ToolInfo>> {
        self.tools_of(server)
    }

    fn call(&self, server: &str, tool: &str, args: Value) -> Result<McpResult> {
        self.call_tool(server, tool, args)
    }

    fn shutdown(&self) {
        let mut servers = self.servers.lock().unwrap_or_else(|e| e.into_inner());
        for (_, server) in servers.drain() {
            let mut server = server.lock().unwrap_or_else(|e| e.into_inner());
            server.shutdown();
        }
    }
}

impl Drop for SyncMcpBridge {
    fn drop(&mut self) {
        self.shutdown();
    }
}
