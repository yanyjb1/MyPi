//! Conversation entries — the **data model**, shared by the store and the
//! TUI. Deliberately free of rendering dependencies (no ratatui, no
//! Palette): both the SQLite persistence layer and the view layer consume
//! this enum, so it must live below both (this reverses the old
//! store→tui reverse dependency, flaw A in ARCHITECTURE.md).
//!
//! Rendering dispatches on these kinds (`tui::zone::main::history::render`);
//! persistence (`crate::store`) serializes them via to/from_payload.

/// Stand-in for an assistant reply that arrived empty.
///
/// Lives here because **two writers must agree on it byte-for-byte**: the
/// session writes it into the reply entry it persists, and the agent loop
/// writes it into the live context it sends. When they disagreed (the loop
/// sent `content: null`, the store held this placeholder) the live
/// conversation and its replay differed, so a restart changed the bytes the
/// model saw and the prefix cache went cold.
pub const EMPTY_REPLY: &str = "(无输出)";

/// One item on the model's todo list.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TodoTask {
    pub content: String,
    pub status: TodoStatus,
    /// Why it is blocked (`block` op carries it). Absent otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
}

/// Where a todo item is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    /// Not started.
    Pending,
    /// Being worked on right now.
    InProgress,
    /// Finished.
    Done,
    /// Cannot proceed; `blocker` says why.
    Blocked,
    /// Deliberately dropped (not the same as "not done yet").
    Abandoned,
}

impl TodoStatus {
    /// The word the tool sends back to the model (`view` output and summaries).
    pub fn as_str(self) -> &'static str {
        match self {
            TodoStatus::Pending => "pending",
            TodoStatus::InProgress => "in_progress",
            TodoStatus::Done => "done",
            TodoStatus::Blocked => "blocked",
            TodoStatus::Abandoned => "abandoned",
        }
    }

    pub fn parse(s: &str) -> Option<TodoStatus> {
        match s {
            "pending" => Some(TodoStatus::Pending),
            "in_progress" => Some(TodoStatus::InProgress),
            "done" => Some(TodoStatus::Done),
            "blocked" => Some(TodoStatus::Blocked),
            "abandoned" => Some(TodoStatus::Abandoned),
            _ => None,
        }
    }
}

/// A named group of tasks. Phases are what keeps a long list readable — and
/// what lets the model say "phase 2 is done" without touching phase 1.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TodoPhase {
    pub name: String,
    pub tasks: Vec<TodoTask>,
}

impl TodoPhase {
    /// How many tasks in this phase are done.
    pub fn done(&self) -> usize {
        self.tasks
            .iter()
            .filter(|t| t.status == TodoStatus::Done)
            .count()
    }
}

/// Count `(done, total)` across every phase.
pub fn todo_counts(phases: &[TodoPhase]) -> (usize, usize) {
    let total = phases.iter().map(|p| p.tasks.len()).sum();
    let done = phases.iter().map(|p| p.done()).sum();
    (done, total)
}

// One history entry: the four message kinds plus a session-level error.
//
// Persisted to the SQLite `entries` table: `seq` monotonically increasing from 1,
// `kind` stored as text, `payload` as JSON. The render layer only knows this enum —
// no string-prefix contracts anymore.
// The wire contract: this enum IS the protocol data model — the daemon ships
// entries to front ends verbatim (`wire::ServerMsg::Transcript` / `::Entry`),
// so the serde representation below is load-bearing. Do not rename fields
// without bumping `wire::PROTO_VERSION`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
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
    //
    // `text` and `first` preserve the **wire shape** the model actually sent.
    // One assistant message may carry text *and* several tool calls
    // (`Assistant { content: Some("我先查一下"), tool_calls: [c1, c2] }`);
    // replaying that as disconnected per-call messages would change the bytes
    // the gateway sees. `first` marks the message's opening call (true) so
    // replay can group a run of calls back into one message; `text` rides
    // that opening call (empty on the rest).
    ToolRequest {
        call_id: String,
        name: String,
        args: String,
        intent: String,
        text: String,
        first: bool,
    },
    // Tool call result. `ok` decides the card color.
    // `result` is the **exact text sent to the model**.
    //
    // `details` is the tool's **structured payload for front ends**: the data a
    // renderer draws from (which lines changed, the exit code, hit count, ...)
    // instead of re-parsing prose. Opaque here — the core never reads it, the
    // tool shapes it, each UI interprets it on its own. Persisted and shipped
    // verbatim, so a resumed session and a second front end see the same thing
    // the live one saw; `None` for tools with nothing structured to say (and for
    // rows written before this field existed).
    //
    // `duration_ms` is measured by the loop around the call, so it exists for
    // every tool whether or not the tool tracks time.
    ToolResult {
        call_id: String,
        name: String,
        ok: bool,
        result: String,
        #[serde(default)]
        details: Option<serde_json::Value>,
        #[serde(default)]
        duration_ms: u64,
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
    // the chat renderer learning about it.
    //
    // **Never persisted** (the old comment here claimed the opposite — it was
    // wrong): notices are local narration for the person watching, they never
    // enter a request, so they reach the transcript through `SessionState::echo`
    // (memory only). "What the model saw" lives in the entries table; "what we
    // told the user about ourselves" does not.
    //
    // `pin`: only a System notice may declare itself **pinned**. A pinned
    // notice always renders at the head of the message array (first-come
    // order among pinned ones) whenever the viewport covers the array top —
    // but it is a normal block otherwise: wheeling up scrolls it out of
    // view like anything else. It never owns the rows, it owns its slot.
    // Unpinned by default; every other kind queues in arrival order, no
    // exceptions.
    System {
        text: String,
        align: Align,
        pin: bool,
    },
    // The model's **todo list**, as a first-class piece of session state.
    //
    // Not narration: this entry exists so the list survives things the
    // conversation does not — a resume, a branch switch, a compaction. The
    // effective list is the **last** `Todo` entry; the tool that maintains it
    // sends *operations* (`done`, `start`, …) and the model only has to
    // remember the one item it just wrote, which is the whole point of keeping
    // a list outside its head.
    //
    // It renders as nothing: the visible record of a `todo` call is that call's
    // own card (its `details` carry the full list at that moment). Same shape as
    // [`Entry::Name`] — a marker the transcript carries, not a thing it says.
    Todo {
        phases: Vec<TodoPhase>,
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

// Recover the model-facing text of a legacy tool-result row that stored a
// *view* instead of the text (see `from_payload`).
//
// The view is a UI concept — this module deliberately does not know its type;
// it reads the two shapes it can see in old rows and nothing else. Unknown
// shapes yield `None`, which degrades to an empty result rather than a panic.
fn legacy_view_text(view: Option<&serde_json::Value>) -> Option<String> {
    let view = view?;
    if let Some(text) = view.get("Plain").and_then(|p| p.get("text")).and_then(|t| t.as_str()) {
        return Some(text.to_string());
    }
    let diff = view.get("Diff")?;
    let mut out = String::new();
    for d in diff.get("deletions").and_then(|d| d.as_array()).into_iter().flatten() {
        out.push_str("- ");
        out.push_str(d.as_str().unwrap_or_default());
        out.push('\n');
    }
    for i in diff.get("insertions").and_then(|i| i.as_array()).into_iter().flatten() {
        out.push_str("+ ");
        out.push_str(i.as_str().unwrap_or_default());
    }
    Some(out)
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
    pub fn usage_summary(u: &crate::server::ai::types::Usage) -> UsageSummary {
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
                text,
                first,
            } => (
                "tool_request",
                serde_json::json!({
                    "call_id": call_id, "name": name, "args": args, "intent": intent,
                    "text": text, "first": first
                })
                .to_string(),
            ),
            Entry::ToolResult {
                call_id,
                name,
                ok,
                result,
                details,
                duration_ms,
            } => (
                "tool_result",
                serde_json::json!({
                    "call_id": call_id, "name": name, "ok": ok, "result": result,
                    "details": details, "duration_ms": duration_ms
                })
                .to_string(),
            ),
            Entry::Error { text } => ("error", serde_json::json!({ "text": text }).to_string()),
            Entry::Name { name } => ("name", serde_json::json!({ "name": name }).to_string()),
            Entry::Todo { phases } => (
                "todo",
                serde_json::json!({ "phases": phases }).to_string(),
            ),
            Entry::System { text, align, pin } => (
                "system",
                serde_json::json!({ "text": text, "align": align, "pin": pin }).to_string(),
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
                    // Legacy rows predate these fields: no attached text, and
                    // treat each as its own message (the old replay shape).
                    text: v
                        .get("text")
                        .and_then(|t| t.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    first: v.get("first").and_then(|f| f.as_bool()).unwrap_or(true),
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
                // Legacy DBs (before `result` existed) stored the *view* instead:
                // `{"Plain":{"text":…}}` or `{"Diff":{"deletions":[…],…}}`.
                // Recovered once, here, so the rest of the system only ever sees
                // `result`. Hand-parsed on purpose: the view is a UI concept and
                // this module must not carry a UI type to read old rows.
                result: v
                    .get("result")
                    .and_then(|r| r.as_str())
                    .map(String::from)
                    .unwrap_or_else(|| legacy_view_text(v.get("view")).unwrap_or_default()),
                details: v.get("details").cloned().filter(|d| !d.is_null()),
                duration_ms: v.get("duration_ms").and_then(|d| d.as_u64()).unwrap_or(0),
            },
            "error" => Entry::Error {
                text: v.get("text")?.as_str()?.to_string(),
            },
            "name" => Entry::Name {
                name: v.get("name")?.as_str()?.to_string(),
            },
            "todo" => Entry::Todo {
                phases: serde_json::from_value(v.get("phases")?.clone()).ok()?,
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
                // Older payloads predate `pin`: absent means unpinned.
                pin: v.get("pin").and_then(|p| p.as_bool()).unwrap_or(false),
            },
            _ => return None,
        })
    }
}
