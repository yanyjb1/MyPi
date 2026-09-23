//! The turn runner — one background thread per turn.
//!
//! Mirrors pi's server-side session runner: the surface (TUI or headless
//! caller) hands over a [`TurnRequest`]; the thread drives the agent
//! loop and translates its raw callbacks into protocol
//! [`SessionEvent`]s on the given channel. It knows nothing about
//! terminals.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use crate::agent::loop_rs::{run, LoopConfig};
use crate::ai::client::Client;
use crate::ai::types::Context as ChatContext;
use crate::entry::Entry;
use crate::server::events::SessionEvent;

/// Everything one turn needs. Built by the surface; consumed by
/// [`spawn_turn`].
pub struct TurnRequest {
    pub client: Client,
    // Shared chat history: the turn thread **writes back** the finalized
    // history when done, so the model remembers the previous turn (and
    // prefix-cache hits depend on it).
    pub chat: Arc<Mutex<ChatContext>>,
    pub text: String,
    pub cfg: LoopConfig,
    // Once set, the runner stops reading from the stream and disconnects
    // (a real interrupt; no wasted tokens).
    pub interrupt: Arc<std::sync::atomic::AtomicBool>,
    // Tool-relative paths resolve against the session directory.
    pub cwd: std::path::PathBuf,
    // Shared cwd slot: /cd migrates it between turns.
    pub cwd_slot: Arc<std::sync::RwLock<std::path::PathBuf>>,
}

// The background thread runs one turn. Owns its client and message replica.
pub fn spawn_turn(tx: Sender<SessionEvent>, req: TurnRequest) {
    std::thread::spawn(move || {
        let TurnRequest { client, chat, text, cfg, interrupt, cwd, cwd_slot } = req;
        let send = |ev: SessionEvent| -> bool { tx.send(ev).is_ok() };
        let chat_arc = chat.clone();
        let mut tools = crate::agent::tools::BuiltinTools::new(cwd).with_cwd_slot(cwd_slot);
        // Snapshot for this turn: the lock is held only for the clone, never during network I/O
        let mut chat = chat.lock().expect("chat 锁中毒").clone();
        // Tool manuals ship with the request — without them the model does not know the tools exist
        chat.tools = crate::agent::tools::BuiltinTools::definitions();
        // Callback returning false -> client stops reading and disconnects (a real interrupt; no wasted tokens)
        let r = run(&client, &mut chat, &text, &cfg, &mut tools, |delta| {
            let _ = send(SessionEvent::Delta(delta.to_string()));
            !interrupt.load(std::sync::atomic::Ordering::Relaxed)
        }, |r| {
            let _ = send(SessionEvent::ReasoningDelta(r.to_string()));
        }, |ev| {
            let _ = send(match ev {
                crate::agent::loop_rs::ToolEvent::Start { call_id, name, args, intent } =>
                    SessionEvent::ToolStart { call_id, name, args, intent },
                crate::agent::loop_rs::ToolEvent::Finish { call_id, name, ok, result } =>
                    SessionEvent::ToolFinish { call_id, name, ok, result },
            });
        });
        match r {
            Ok(outcome) => {
                if outcome.hit_round_limit {
                    let _ = send(SessionEvent::Error(format!(
                        "工具调用轮数撞上限（{}），被强制收工",
                        cfg.max_rounds
                    )));
                }
                let _ =
                    send(SessionEvent::TurnDone(outcome.message.usage, outcome.message.stop_reason));
                // Write the finalized history back to the shared slot: the model remembers
                // this turn next round (and prefix-cache hits depend on it). Not written on Err —
                // never pollute the shared slot with a partial history.
                *chat_arc.lock().expect("chat 锁中毒") = chat.clone();
                // Whole turn finalized: user + (assistant.tool_calls + tool results) * N + assistant.
                // Persisted in one shot by the session (Commit).
                let _ = send(SessionEvent::Commit(collect_turn(&chat, &text)));
            }
            Err(e) => {
                let _ = send(SessionEvent::Error(format!("{e:#}")));
            }
        }
        let _ = send(SessionEvent::Done);
    });
}

// ---- turn assembly ------------------------------------------------------

/// Rebuild the **full protocol messages** from projected entries (the
/// inverse of [`collect_turn`]). Tool call details (call_id / arguments /
/// results) are all in the DB — the live build, resume, and tree
/// navigation all read the same source, so the model sees the history
/// exactly as it did the first time.
///
/// Dangling safety: if the projection ends inside a tool chain (leaf on
/// a ToolRequest with no matching ToolResult, or vice versa), the tail
/// is repaired — a request without results is dropped together with its
/// pending calls (never a half-open tool_calls message), so the next
/// `run()` always starts from a protocol-legal boundary.
pub fn entries_to_context(entries: &[Entry]) -> ChatContext {
    let mut rebuilt = ChatContext::new().push(crate::ai::types::Message::System {
        content: "你是一个简洁的编程助手。用中文回答。".into(),
    });
    // Pair requests with their results first: call_id -> (ok, result)
    use std::collections::BTreeMap;
    let mut results: BTreeMap<String, (bool, String)> = BTreeMap::new();
    for e in entries {
        if let Entry::ToolResult { call_id, ok, result, .. } = e {
            results.insert(call_id.clone(), (*ok, result.clone()));
        }
    }
    let mut served: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for e in entries {
        match e {
            Entry::User { content } => {
                rebuilt = rebuilt.push(crate::ai::types::Message::User { content: content.clone() });
            }
            Entry::Assistant { content, .. } => {
                rebuilt = rebuilt.push(crate::ai::types::Message::Assistant {
                    content: Some(content.clone()),
                    tool_calls: Vec::new(),
                });
            }
            Entry::ToolRequest { call_id, name, args, .. } => {
                // `args` is replayed verbatim: the model must see the call it
                // actually made, not a reconstruction.
                let call = crate::ai::types::ToolCall {
                    id: call_id.clone(),
                    kind: "function".into(),
                    function: crate::ai::types::FunctionCall {
                        name: name.clone(),
                        arguments: args.clone(),
                    },
                };
                rebuilt = rebuilt.push(crate::ai::types::Message::Assistant {
                    content: None,
                    tool_calls: vec![call],
                });
            }
            Entry::ToolResult { call_id, result, .. } => {
                // The stored result is exactly what the model received
                // back then — use it verbatim.
                served.insert(call_id.clone());
                rebuilt = rebuilt.push(crate::ai::types::Message::Tool {
                    tool_call_id: call_id.clone(),
                    content: result.clone(),
                });
            }
            // System notices and Name markers are UI/persistence metadata:
            // neither has a protocol role.
            Entry::Error { .. } | Entry::Name { .. } | Entry::System { .. } => {}
        }
    }
    // Repair pass: drop trailing requests whose results never arrived
    // (dangling leaf). Walk backwards while the tail is ToolRequest-
    // without-result or a Tool message whose request was dropped.
    loop {
        match rebuilt.messages.last() {
            Some(crate::ai::types::Message::Tool { tool_call_id, .. }) if !served.is_empty() => {
                // A Tool result always pairs with the preceding request;
                // by construction requests come before results, so this
                // cannot dangle. Stop when we hit anything else.
                let id = tool_call_id.clone();
                // Remove this Tool message and its (already emitted) request
                // stays — a result with request is protocol-legal. Nothing
                // to repair.
                let _ = id;
                break;
            }
            Some(crate::ai::types::Message::Assistant { content: None, tool_calls }) if !tool_calls.is_empty() => {
                // Pure tool-call round with no results yet: dangling.
                // Rewind to before this message.
                rebuilt.messages.pop();
                // Also remove the matching result markers (none here by
                // construction) and continue checking the new tail.
                continue;
            }
            _ => break,
        }
    }
    rebuilt
}

/// Whole-turn assembly from the finalized chat replica: user +
/// (assistant.tool_calls + tool results) \* N + final assistant.
pub(crate) fn collect_turn(chat: &ChatContext, text: &str) -> Vec<Entry> {
    use crate::ai::types::Message;
    // The turn starts at the last User message (pushed at the top of run()).
    // Walk **forward** from there — the old "scan backward then reverse" approach
    // inverted each request -> result pair into result -> request.
    let start = chat
        .messages
        .iter()
        .rposition(|m| matches!(m, Message::User { .. }))
        .expect("本轮一定 push 过 User");
    let mut out = Vec::new();
    for m in &chat.messages[start..] {
        match m {
            Message::User { content } => {
                out.push(Entry::User { content: content.clone() });
            }
            Message::Assistant { content, tool_calls } => {
                let c = content.clone().unwrap_or_default();
                if !tool_calls.is_empty() {
                    for tc in tool_calls {
                        // Keep the raw arguments **verbatim**: the card renders
                        // them and resume replays them, so any extraction here
                        // would lose information (an earlier version stored
                        // only `path`, which blanked every bash card and
                        // mis-replayed non-JSON paths as arguments).
                        out.push(Entry::ToolRequest {
                            call_id: tc.id.clone(),
                            name: tc.function.name.clone(),
                            args: tc.function.arguments.clone(),
                            intent: tc
                                .function
                                .arguments_json()
                                .ok()
                                .and_then(|v| v.get("intent").and_then(|i| i.as_str()).map(String::from))
                                .unwrap_or_default(),
                        });
                    }
                    // The tool_calls assistant appears only as a request card;
                    // no duplicate Assistant entry (its content is usually empty)
                } else {
                    out.push(Entry::Assistant { content: c, usage: None, reasoning: None });
                }
            }
            Message::Tool { tool_call_id, content } => {
                // name/call_id backfill from the nearest preceding request card with the same name
                // (pairing unchanged: tool_call_id is the protocol key, name is display-only)
                let name = out
                    .iter()
                    .rev()
                    .find_map(|e| match e {
                        Entry::ToolRequest { call_id, name, .. } if call_id == tool_call_id.as_str() => {
                            Some(name.clone())
                        }
                        _ => None,
                    })
                    .unwrap_or_default();
                out.push(Entry::ToolResult {
                    call_id: tool_call_id.clone(),
                    name,
                    ok: true,
                    result: content.clone(),
                });
            }
            Message::System { .. } => {}
        }
    }
    // Sanity: the first extracted entry must be User (guards against misalignment).
    // text is not compared — it is trimmed input and may differ in whitespace from chat.
    let _ = text;
    debug_assert!(out.first().is_some_and(|e| matches!(e, Entry::User { .. })));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuild_handles_complete_and_dangling_tool_tails() {
        // Complete chain: request + result survive.
        let complete = vec![
            Entry::User { content: "q".into() },
            Entry::ToolRequest { call_id: "c1".into(), name: "bash".into(), args: "{}".into(), intent: String::new() },
            Entry::ToolResult { call_id: "c1".into(), name: "bash".into(), ok: true, result: "out".into() },
            Entry::Assistant { content: "done".into(), usage: None, reasoning: None },
        ];
        let ctx = entries_to_context(&complete);
        assert!(matches!(ctx.messages[1], crate::ai::types::Message::User { .. }));
        assert!(matches!(&ctx.messages[2], crate::ai::types::Message::Assistant { tool_calls, .. } if tool_calls.len() == 1));
        assert!(matches!(ctx.messages[3], crate::ai::types::Message::Tool { .. }));

        // Dangling request tail: dropped (never a half-open tool_calls).
        let dangling = vec![
            Entry::User { content: "q".into() },
            Entry::ToolRequest { call_id: "c2".into(), name: "bash".into(), args: "{}".into(), intent: String::new() },
        ];
        let ctx = entries_to_context(&dangling);
        assert_eq!(ctx.messages.len(), 2); // system + user
    }
}
