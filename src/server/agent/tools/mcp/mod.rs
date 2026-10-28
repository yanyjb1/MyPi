//! The static `mcp` tool: list configured servers/tools or call one tool.

mod bridge;
mod stdio;
mod sync;

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::spec::{Args, Env, Kind, Param, Tool, ToolOutput, ToolProgress};
pub use bridge::{McpBridge, McpContent, McpResult, ServerInfo, ToolInfo};
pub use sync::SyncMcpBridge;

const PARAMS: &[Param] = &[
    Param::req(
        "op",
        Kind::Choice(&["list", "call"]),
        "Operation: list servers/tools or call one tool.",
    ),
    Param::opt("server", Kind::Text, "Configured server name."),
    Param::opt("tool", Kind::Text, "Tool name for call."),
    Param::opt("args", Kind::Any, "Tool arguments as a JSON object."),
];

pub const TOOL: Tool = Tool {
    name: "mcp",
    about: "MCP: list configured servers and tools, or call one tool.",
    params: PARAMS,
    run,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let Some(bridge) = env.mcp.as_deref() else {
        bail!("mcp is not configured");
    };
    let op = args.choice(0)?;
    match op.as_str() {
        "list" => list(bridge, args.opt_str(1)?),
        "call" => call(bridge, args),
        _ => unreachable!("Args::choice validates op"),
    }
}

fn list(bridge: &dyn McpBridge, server: Option<String>) -> Result<ToolOutput> {
    if let Some(server) = server.filter(|server| !server.trim().is_empty()) {
        return Ok(ToolOutput::text(server_block(
            &server,
            &bridge.tools(&server)?,
        )));
    }
    let servers = bridge.servers();
    if servers.is_empty() {
        return Ok(ToolOutput::text("(no MCP servers configured)"));
    }
    let mut out = String::new();
    for server in servers {
        if !out.is_empty() {
            out.push('\n');
        }
        match bridge.tools(&server.name) {
            Ok(tools) => out.push_str(&server_block(&server.name, &tools)),
            Err(error) => out.push_str(&format!("server: {}\n  (error: {error:#})", server.name)),
        }
    }
    Ok(ToolOutput::text(out))
}

fn server_block(server: &str, tools: &[ToolInfo]) -> String {
    let mut out = format!("server: {server}");
    if tools.is_empty() {
        out.push_str("\n  (no tools)");
        return out;
    }
    for tool in tools {
        out.push_str(&format!("\n  {} — {}", tool.name, tool.description));
        if !tool.required.is_empty() {
            out.push_str(&format!(" [required: {}]", tool.required.join(", ")));
        }
    }
    out
}

fn call(bridge: &dyn McpBridge, args: &Args) -> Result<ToolOutput> {
    let server = args
        .opt_str(1)?
        .filter(|server| !server.trim().is_empty())
        .ok_or_else(|| anyhow!("missing required argument(s): server"))?;
    let tool = args
        .opt_str(2)?
        .filter(|tool| !tool.trim().is_empty())
        .ok_or_else(|| anyhow!("missing required argument(s): tool"))?;
    let arguments = match args.opt_any(3) {
        None => json!({}),
        Some(value) if value.is_null() => json!({}),
        Some(value) if value.is_object() => value.clone(),
        Some(_) => bail!("`args` must be a JSON object"),
    };
    let result = bridge.call(&server, &tool, arguments)?;
    let text = render(&result);
    if result.is_error {
        if text.is_empty() {
            bail!("MCP tool returned an error");
        }
        bail!("{text}");
    }
    Ok(ToolOutput::text(text))
}

fn render(result: &McpResult) -> String {
    if result.content.is_empty() {
        return "(no content)".to_string();
    }
    let mut out = String::new();
    for content in &result.content {
        if !out.is_empty() {
            out.push('\n');
        }
        match content {
            McpContent::Text(text) => out.push_str(text),
            McpContent::Unsupported { kind } => {
                out.push_str(&format!("[unsupported content: {kind}]"));
            }
        }
    }
    out
}
