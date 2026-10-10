//! context: inspect the calls this session has already recorded. Read-only: it runs nothing.

use std::sync::Arc;

use anyhow::{Context as _, Result};

use super::spec::{Args, Env, Kind, Param, Tool, ToolOutput, ToolProgress};
use crate::server::agent::request::message::Message;
use crate::server::storage::{base36, parse_base36};

const ANCHOR: usize = 0;
const BEFORE: usize = 1;
const AFTER: usize = 2;
const INCLUDE_USER: usize = 3;
const INCLUDE_RESULTS: usize = 4;

/// The window to look at: the messages settled so far, plus the `cwd` moves. The caller reads
/// storage and hands it in; the tool never touches the database.
#[derive(Default)]
pub struct Window {
    pub messages: Arc<Vec<Message>>,
    pub cwd_trail: Arc<Vec<(i64, String)>>,
}

pub(super) const PARAMS: &[Param] = &[
    Param::req(
        "anchor",
        Kind::Text,
        "Anchor: an artifact reference (`#3xl`) or some tool result's call_id.",
    ),
    Param::df(
        "before",
        Kind::Number,
        "Tool calls to show before the anchor. Default 10.",
        "10",
    ),
    Param::df(
        "after",
        Kind::Number,
        "Tool calls to show after the anchor. Default 10.",
        "10",
    ),
    Param::df(
        "include_user",
        Kind::Flag,
        "Also list user messages. Default false.",
        "false",
    ),
    Param::df(
        "include_results",
        Kind::Flag,
        "Append each call's full result instead of listing calls only. Default false.",
        "false",
    ),
];

pub(super) const TOOL: Tool = Tool {
    name: "context",
    about: "Query this session's recorded tool calls (read-only; nothing is executed). Mainly for self-inspection after\n\
            compaction: where artifact `#<ref>` came from — pass anchor=`#<ref>` (or a tool result's call_id) and get the\n\
            surrounding call sequence, cwd changes included. anchor may be a call_id itself; before/after control how\n\
            many calls on each side; include_user=true also lists user messages (to see how the request moved);\n\
            include_results=true appends each call's full result (to see errors).",
    params: PARAMS,
    run,
};

fn run(
    env: &mut Env,
    args: &Args,
    _progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    let anchor = args.str(ANCHOR)?;
    let before = args.int(BEFORE)?.max(0) as usize;
    let after = args.int(AFTER)?.max(0) as usize;
    let include_user = args.flag(INCLUDE_USER)?;
    let include_results = args.flag(INCLUDE_RESULTS)?;

    let window = &env.inbox.window;
    let out = query(
        &window.messages,
        &window.cwd_trail,
        &Query {
            anchor,
            before,
            after,
            include_user,
            include_results,
        },
    )?;
    Ok(ToolOutput::text(out))
}

/// What one query needs.
struct Query {
    anchor: String,
    before: usize,
    after: usize,
    include_user: bool,
    include_results: bool,
}

/// Find the anchor — an artifact ref like `#3xl`, or a bare `call_id` — in the settled history and
/// draw the calls around it, marking `cwd` moves. Read-only: runs nothing.
fn query(
    history: &[Message],
    cwd_trail: &[(i64, String)],
    q: &Query,
) -> Result<String> {
    let anchor = q.anchor.trim();

    // Anchor → an index: `call_id` matches a ToolRequest/ToolResult; `#<ref>` matches any call
    // whose **result text** mentions that artifact.
    let anchor_idx = if let Some(word) = anchor.strip_prefix('#') {
        let id = parse_base36(&word.trim().to_ascii_lowercase())
            .with_context(|| format!("artifact reference must be #<ref>; got {anchor}"))?;
        let needle = format!("#{}", base36(id));
        history
            .iter()
            .position(|e| matches!(e, Message::ToolResult { result, .. } if result.contains(&needle)))
            .with_context(|| format!("no record of artifact {needle} being produced"))?
    } else {
        history
            .iter()
            .position(|e| match e {
                Message::ToolRequest { call_id, .. } | Message::ToolResult { call_id, .. } => {
                    call_id == anchor
                }
                _ => false,
            })
            .with_context(|| format!("no call_id {anchor} in this history"))?
    };

    // The lines to print: the anchor, N calls either side, user messages per the switches.
    let mut picks: Vec<usize> = Vec::new();
    for (i, e) in history.iter().enumerate() {
        let is_tool = matches!(e, Message::ToolRequest { .. } | Message::ToolResult { .. });
        let is_user = matches!(e, Message::User { .. });
        let in_window = i >= anchor_idx.saturating_sub(q.before)
            && i <= anchor_idx.saturating_add(q.after);
        if in_window && (is_tool || (q.include_user && is_user)) {
            picks.push(i);
        }
    }
    let mut out = String::new();
    // `cwd` trail as (seq, path); a crossing prints one "cwd: …" line.
    let mut trail = cwd_trail.iter().peekable();
    for &i in &picks {
        while let Some((_seq, path)) = trail
            .peek()
            .map(|(s, p)| (*s, p.clone()))
            .filter(|(s, _)| *s <= i as i64)
        {
            out.push_str(&format!("cwd: {path}\n"));
            trail.next();
        }
        match &history[i] {
            Message::User { content, .. } => {
                out.push_str(&format!("user: {}\n", first_line(content)));
            }
            Message::ToolRequest {
                call_id,
                name,
                args,
                ..
            } => {
                out.push_str(&format!(
                    "#{i} call {name} ({call_id}): {}\n",
                    first_line(args)
                ));
            }
            Message::ToolResult {
                call_id,
                name,
                ok,
                result,
                ..
            } => {
                if q.include_results {
                    out.push_str(&format!(
                        "#{} result {} ({}):\n{}\n",
                        i,
                        name,
                        if *ok { "ok" } else { "FAILED" },
                        result
                    ));
                } else {
                    out.push_str(&format!(
                        "#{i} result {name} ({}) {}\n",
                        call_id,
                        if *ok { "ok" } else { "FAILED" }
                    ));
                }
            }
            _ => {}
        }
    }
    if out.is_empty() {
        out.push_str("(no tool calls in this window — try widening before/after)");
    }
    Ok(out)
}

/// First line, truncated.
fn first_line(s: &str) -> String {
    let line = s.lines().next().unwrap_or("");
    line.chars().take(120).collect()
}
