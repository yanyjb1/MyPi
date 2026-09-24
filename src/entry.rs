//! Conversation entries — the **data model**, shared by the store and the
//! TUI. Deliberately free of rendering dependencies (no ratatui, no
//! Palette): both the SQLite persistence layer and the view layer consume
//! this enum, so it must live below both (this reverses the old
//! store→tui reverse dependency, flaw A in ARCHITECTURE.md).
//!
//! Rendering (`crate::tui::components::chat`) dispatches on these kinds;
//! persistence (`crate::store`) serializes them via to/from_payload.

// One history entry: the four message kinds plus a session-level error.
//
// Persisted to the SQLite `entries` table: `seq` monotonically increasing from 1,
// `kind` stored as text, `payload` as JSON. The render layer only knows this enum —
// no string-prefix contracts anymore.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    // A user message.
    User {
        content: String,
    },
    // Model reply (final). `usage` feeds the stats line; optional.
    // Persisted verbatim; resume replays it into the protocol message
    // byte-identical. The thinking chain is a **separate** entry
    // ([`Entry::Reasoning`]) — display toggles belong to that block.
    Assistant {
        content: String,
        usage: Option<UsageSummary>,
    },
    // The thinking chain, **its own entry/block** (split from Assistant:
    // one block per display unit means the cache never renders the same
    // block in two forms). Display + persistence only; never sent back
    // in requests. Ordered immediately before its Assistant reply.
    Reasoning {
        content: String,
    },
    // Tool call request: the model named a tool.
    //
    // `args` is the **raw JSON argument string** exactly as the model sent it —
    // the card renders it (syntax-highlighted) and resume replays it verbatim
    // into the protocol message, so a rebuilt history is byte-identical.
    // `intent` is the model's own one-line "what am I about to do", shown in the
    // live slot while the (blocking) tool runs.
    // `call_id` is the protocol pairing key.
    ToolRequest {
        call_id: String,
        name: String,
        args: String,
        intent: String,
    },
    // Tool call result. `ok` decides the card color.
    // `result` is the **exact text sent to the model** — the only thing persisted;
    // the rendering (view) is **synthesized at render time** from (name, ok, result), never stored.
    ToolResult {
        call_id: String,
        name: String,
        ok: bool,
        result: String,
    },
    // Session-level errors (HTTP failures, round-limit brakes...). Not persisted; memory stream only.
    Error {
        text: String,
    },
    // Session name marker on the conversation tree (pi's session_info). Persisted;
    // the effective name is the nearest `name` entry looking back from the leaf,
    // so branches inherit the name and renaming only affects the current branch.
    Name {
        name: String,
    },
    // A system notice (`/model` switched, context compacted, `/resume` restored...).
    //
    // Unlike the fixed kinds above, the **emitter chooses the rendering**: it
    // constructs this entry with the alignment it wants, so a future feature
    // (context-compaction reports, model switches) can present itself without
    // the chat renderer learning about it. Persisted like everything else.
    System {
        text: String,
        align: Align,
    },
    // A context compaction marker — the **conversation fork point**.
    //
    // Semantically it is a branch node on the entry tree: everything
    // before `first_kept_seq` is superseded by `summary` (kept verbatim
    // in the payload for audit; the live context rebuilds from
    // first_kept_seq onward). Tokens before/after are *computed*, never
    // stored — the CPU is good at arithmetic.
    //
    // entries_to_context treats this as the context root: system + a
    // synthesized summary user turn + entries after the marker.
    Compaction {
        // First entry seq of the retained (verbatim) region.
        first_kept_seq: usize,
        // The compaction summary (the compacted region's stand-in).
        summary: String,
    },
}

// How a [`Entry::System`] notice lines itself up. The emitter picks; the
// renderer only obeys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Align {
    Left,
    Center,
}

// The tool result's view for the UI. **The model only ever receives plain text**;
// this enum only declares how the UI draws it — no protocol role, **never persisted** —
// synthesized at render time by `synthesize` from (tool name, ok, result text).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ToolView {
    // Plain text, no markdown rendering, auto-folded beyond 5 lines.
    Plain {
        text: String,
    },
    // Line diff: deletions on red, insertions on green (edit tool).
    Diff {
        deletions: Vec<String>,
        insertions: Vec<String>,
    },
}

impl ToolView {
    // Synthesize the UI presentation from data (tool name / ok / model-facing text).
    //
    // The single source of the synthesis rules. Only the result text is persisted;
    // the view is a derived UI concept and takes no part in persistence.
    pub fn synthesize(name: &str, ok: bool, result: &str) -> ToolView {
        if !ok {
            return ToolView::Plain {
                text: result.to_string(),
            };
        }
        match name {
            "edit" | "mass_edit" => {
                // Diff-shaped text (- / + lines) splits into a Diff; otherwise plain text
                let mut deletions = Vec::new();
                let mut insertions = Vec::new();
                for line in result.lines() {
                    if let Some(rest) = line.strip_prefix("- ") {
                        deletions.push(rest.to_string());
                    } else if let Some(rest) = line.strip_prefix("+ ") {
                        insertions.push(rest.to_string());
                    }
                }
                if deletions.is_empty() && insertions.is_empty() {
                    ToolView::Plain {
                        text: result.to_string(),
                    }
                } else {
                    ToolView::Diff {
                        deletions,
                        insertions,
                    }
                }
            }
            _ => ToolView::Plain {
                text: result.to_string(),
            },
        }
    }
}

// Fields the stats line needs (the minimal set extracted from usage).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UsageSummary {
    pub total_tokens: u64,
    pub prompt_tokens: u64,
    pub cached_tokens: u64,
    pub completion_tokens: u64,
    pub reasoning_tokens: u64,
}

impl Entry {
    // Build a stats summary from the ai layer's usage.
    pub fn usage_summary(u: &crate::ai::types::Usage) -> UsageSummary {
        UsageSummary {
            total_tokens: u.total_tokens,
            prompt_tokens: u.prompt_tokens,
            cached_tokens: u.cached_tokens.unwrap_or(0),
            completion_tokens: u.completion_tokens,
            reasoning_tokens: u.reasoning_tokens.unwrap_or(0),
        }
    }

    // Serialize to (kind, payload_json). Used for DB writes.
    pub fn to_payload(&self) -> (&'static str, String) {
        match self {
            Entry::User { content } => (
                "user",
                serde_json::json!({ "content": content }).to_string(),
            ),
            Entry::Assistant { content, usage } => (
                "assistant",
                serde_json::json!({ "content": content, "usage": usage }).to_string(),
            ),
            Entry::Reasoning { content } => (
                "reasoning",
                serde_json::json!({ "content": content }).to_string(),
            ),
            Entry::ToolRequest {
                call_id,
                name,
                args,
                intent,
            } => (
                "tool_request",
                serde_json::json!({
                    "call_id": call_id, "name": name, "args": args, "intent": intent
                })
                .to_string(),
            ),
            Entry::ToolResult {
                call_id,
                name,
                ok,
                result,
            } => (
                "tool_result",
                serde_json::json!({ "call_id": call_id, "name": name, "ok": ok, "result": result })
                    .to_string(),
            ),
            Entry::Error { text } => ("error", serde_json::json!({ "text": text }).to_string()),
            Entry::Name { name } => ("name", serde_json::json!({ "name": name }).to_string()),
            Entry::System { text, align } => (
                "system",
                serde_json::json!({ "text": text, "align": align }).to_string(),
            ),
            Entry::Compaction {
                first_kept_seq,
                summary,
            } => (
                "compaction",
                serde_json::json!({
                    "first_kept_seq": first_kept_seq, "summary": summary
                })
                .to_string(),
            ),
        }
    }

    // Restore from (kind, payload_json). Used for DB reads.
    pub fn from_payload(kind: &str, payload: &str) -> Option<Entry> {
        let v: serde_json::Value = serde_json::from_str(payload).ok()?;
        Some(match kind {
            "user" => Entry::User {
                content: v.get("content")?.as_str()?.to_string(),
            },
            // Current format: reasoning is its own entry. A payload still
            // carrying `reasoning` is a legacy row — replay the content and
            // drop the chain (no old-DB migration is owed).
            "assistant" => Entry::Assistant {
                content: v.get("content")?.as_str()?.to_string(),
                usage: v
                    .get("usage")
                    .and_then(|u| serde_json::from_value(u.clone()).ok()),
            },
            "reasoning" => Entry::Reasoning {
                content: v.get("content")?.as_str()?.to_string(),
            },
            "tool_request" => {
                // `args` is the current key; `object` is the pre-refactor one
                // (it held a bare path, and resume mis-replayed it as JSON —
                // reading it keeps old sessions loadable, empty args is the
                // honest value for a card we cannot reconstruct).
                let args = v
                    .get("args")
                    .and_then(|a| a.as_str())
                    .or_else(|| v.get("object").and_then(|o| o.as_str()))
                    .unwrap_or_default()
                    .to_string();
                Entry::ToolRequest {
                    call_id: v
                        .get("call_id")
                        .and_then(|c| c.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    name: v.get("name")?.as_str()?.to_string(),
                    args,
                    intent: v
                        .get("intent")
                        .and_then(|i| i.as_str())
                        .unwrap_or_default()
                        .to_string(),
                }
            }
            "tool_result" => Entry::ToolResult {
                call_id: v
                    .get("call_id")
                    .and_then(|c| c.as_str())
                    .unwrap_or_default()
                    .to_string(),
                name: v.get("name")?.as_str()?.to_string(),
                ok: v.get("ok")?.as_bool()?,
                // Legacy DBs store view without result: recover from view once (migration path)
                result: v
                    .get("result")
                    .and_then(|r| r.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| {
                        let view = v.get("view").cloned();
                        match view.and_then(|view| serde_json::from_value::<ToolView>(view).ok()) {
                            Some(ToolView::Plain { text }) => text,
                            Some(ToolView::Diff {
                                deletions,
                                insertions,
                            }) => {
                                let mut t = String::new();
                                for d in &deletions {
                                    t.push_str("- ");
                                    t.push_str(d);
                                    t.push('\n');
                                }
                                for i in &insertions {
                                    t.push_str("+ ");
                                    t.push_str(i);
                                }
                                t
                            }
                            None => String::new(),
                        }
                    }),
            },
            "error" => Entry::Error {
                text: v.get("text")?.as_str()?.to_string(),
            },
            "name" => Entry::Name {
                name: v.get("name")?.as_str()?.to_string(),
            },
            "compaction" => Entry::Compaction {
                first_kept_seq: v
                    .get("first_kept_seq")
                    .and_then(|s| s.as_u64())
                    .map(|s| s as usize)?,
                summary: v.get("summary")?.as_str()?.to_string(),
            },
            "system" => Entry::System {
                text: v.get("text")?.as_str()?.to_string(),
                align: v
                    .get("align")
                    .and_then(|a| serde_json::from_value(a.clone()).ok())
                    .unwrap_or(Align::Left),
            },
            _ => return None,
        })
    }
}
