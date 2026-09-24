//! The turn runner — one background thread per turn.
//!
//! Mirrors pi's server-side session runner: the surface (TUI or headless
//! caller) hands over a [`TurnRequest`]; the thread drives the agent
//! loop and translates its raw callbacks into protocol
//! [`SessionEvent`]s on the given channel. It knows nothing about
//! terminals.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use crate::agent::loop_rs::{LoopConfig, run};
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
    // The finalized history snapshot (pre-turn): feeds the `context`
    // tool — the model's window into how earlier artifacts (#N) came to
    // be. Cheap to clone: only compacted summaries + tool exchanges are
    // sizeable, and they are already resident in memory.
    pub history: Arc<Vec<Entry>>,
    // The cwd migrations recorded so far (seq, path): the `context`
    // tool annotates its output with "where did the cwd move" markers.
    pub cwd_trail: Arc<Vec<(i64, String)>>,
    // Profile tool roster: `None` = everything; `Some(names)` = the
    // profile's allow-list (definitions_for + execute gate both read it).
    pub tool_filter: Option<Vec<String>>,
    // Artifact storage for this session (own DB connection; see
    // `ArtifactStore::open`). `None` = store unavailable — oversized tool
    // output flows into the context verbatim (the pre-artifact behavior).
    pub artifacts: Option<crate::agent::artifacts::ArtifactStore>,
    // Tool-layer knobs (bash timeout, ...) from config.yaml.
    pub tools: crate::agent::tools::ToolsConfig,
}

// The background thread runs one turn. Owns its client and message replica.
pub fn spawn_turn(tx: Sender<SessionEvent>, req: TurnRequest) {
    std::thread::spawn(move || {
        let TurnRequest {
            client,
            chat,
            text,
            cfg,
            interrupt,
            cwd,
            cwd_slot,
            history,
            cwd_trail,
            tool_filter,
            artifacts,
            tools: tools_cfg,
        } = req;
        let send = |ev: SessionEvent| -> bool { tx.send(ev).is_ok() };
        let chat_arc = chat.clone();
        let mut tools = crate::agent::tools::BuiltinTools::new(cwd)
            .with_cwd_slot(cwd_slot)
            .with_history(history, cwd_trail)
            .with_artifacts_opt(artifacts)
            .with_bash_timeout(tools_cfg.bash_timeout_secs)
            .with_enabled(tool_filter);
        // Snapshot for this turn: the lock is held only for the clone, never during network I/O
        let mut chat = chat.lock().expect("chat 锁中毒").clone();
        // Tool manuals ship with the request — without them the model does not know the tools exist
        chat.tools = tools.definitions_for();
        // Callback returning false -> client stops reading and disconnects (a real interrupt; no wasted tokens)
        let r = run(
            &client,
            &mut chat,
            &text,
            &cfg,
            &mut tools,
            |delta| {
                let _ = send(SessionEvent::Delta(delta.to_string()));
                !interrupt.load(std::sync::atomic::Ordering::Relaxed)
            },
            |r| {
                let _ = send(SessionEvent::ReasoningDelta(r.to_string()));
            },
            |ev| {
                let _ = send(match ev {
                    crate::agent::loop_rs::ToolEvent::Start {
                        call_id,
                        name,
                        args,
                        intent,
                        text,
                        first,
                    } => SessionEvent::ToolStart {
                        call_id,
                        name,
                        args,
                        intent,
                        text,
                        first,
                    },
                    crate::agent::loop_rs::ToolEvent::Finish {
                        call_id,
                        name,
                        ok,
                        result,
                    } => SessionEvent::ToolFinish {
                        call_id,
                        name,
                        ok,
                        result,
                    },
                });
            },
        );
        match r {
            Ok(outcome) => {
                if outcome.hit_round_limit {
                    let _ = send(SessionEvent::Error(format!(
                        "工具调用轮数撞上限（{}），被强制收工",
                        cfg.max_rounds
                    )));
                }
                let _ = send(SessionEvent::TurnDone(
                    outcome.message.usage,
                    outcome.message.stop_reason,
                ));
                // Persistence is the session's job now: it stores what the
                // event stream built up (`pending`), not a re-projection of
                // this context. We still write the finalized history back so
                // the **next request** carries it (the model must remember
                // this turn, and the prefix cache keys on it).
                *chat_arc.lock().expect("chat 锁中毒") = chat;
                let _ = text; // retained for signature parity; no longer projected
            }
            Err(e) => {
                let _ = send(SessionEvent::Error(format!("{e:#}")));
            }
        }
        let _ = send(SessionEvent::Done);
    });
}

// ---- turn assembly ------------------------------------------------------

/// Rebuild the **full protocol messages** from projected entries. Tool call
/// details (call_id / arguments / results) are all in the DB — the live
/// build, resume, and tree navigation all read the same source, so the model
/// sees the history exactly as it did the first time.
///
/// Dangling safety: if the projection ends inside a tool chain (leaf on
/// a ToolRequest with no matching ToolResult, or vice versa), the tail
/// is repaired — a request without results is dropped together with its
/// pending calls (never a half-open tool_calls message), so the next
/// `run()` always starts from a protocol-legal boundary.
pub fn entries_to_context(system: &str, entries: &[Entry]) -> ChatContext {
    // A compaction marker is the context root: everything before the **last**
    // one is superseded by that marker's summary. Dropping it here is what
    // makes a resumed /compacted session replay the *compacted* context —
    // the marker's own arm below emits the summary, and the entries ahead of
    // it never reach the model. Without this the pre-compaction region came
    // back on resume / tree navigation and the user paid for the same tokens
    // a second time.
    let start = entries
        .iter()
        .rposition(|e| matches!(e, Entry::Compaction { .. }))
        .unwrap_or(0);
    let entries = &entries[start..];
    let mut rebuilt = ChatContext::new().push(crate::ai::types::Message::System {
        content: system.to_string(),
    });
    // Which calls actually got a result. A call without one is a dangling
    // tail (the leaf sits mid-tool, e.g. an interrupted turn); it must not
    // become a tool_calls message the gateway would reject for having no
    // matching result. Groups are emitted complete-only below.
    let mut complete: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for e in entries {
        if let Entry::ToolResult { call_id, .. } = e {
            complete.insert(call_id.clone());
        }
    }
    let mut i = 0;
    while i < entries.len() {
        match &entries[i] {
            Entry::User { content } => {
                rebuilt = rebuilt.push(crate::ai::types::Message::User {
                    content: content.clone(),
                });
                i += 1;
            }
            // Reasoning never re-enters the protocol: display-only.
            Entry::Assistant { content, .. } => {
                rebuilt = rebuilt.push(crate::ai::types::Message::Assistant {
                    content: Some(content.clone()),
                    tool_calls: Vec::new(),
                });
                i += 1;
            }
            Entry::Reasoning { .. } => i += 1,
            // A run of tool entries is one assistant message (possibly
            // several calls) plus its result messages. Rebuild the **original
            // wire shape**: group the calls the model sent together, emit one
            // Assistant carrying them all, then the Tool messages — not N
            // disconnected assistant/tool pairs, which serialize to different
            // bytes and cost a prefix-cache miss.
            Entry::ToolRequest { .. } | Entry::ToolResult { .. } => {
                let end = entries[i..]
                    .iter()
                    .position(|e| {
                        !matches!(e, Entry::ToolRequest { .. } | Entry::ToolResult { .. })
                    })
                    .map(|p| i + p)
                    .unwrap_or(entries.len());
                let block = &entries[i..end];
                // Split the block into call groups: a `first` request opens
                // one; the rest of its message's calls follow.
                // (text, calls, results) — one entry per original message.
                type Group = (
                    String,
                    Vec<crate::ai::types::ToolCall>,
                    Vec<(String, String)>,
                );
                let mut groups: Vec<Group> = Vec::new();
                for e in block {
                    match e {
                        Entry::ToolRequest {
                            call_id,
                            name,
                            args,
                            text,
                            first,
                            ..
                        } => {
                            if *first || groups.is_empty() {
                                groups.push((text.clone(), Vec::new(), Vec::new()));
                            }
                            if complete.contains(call_id) {
                                let call = crate::ai::types::ToolCall {
                                    id: call_id.clone(),
                                    kind: "function".into(),
                                    function: crate::ai::types::FunctionCall {
                                        name: name.clone(),
                                        // `args` replays verbatim: the model
                                        // must see the call it actually made.
                                        arguments: args.clone(),
                                    },
                                };
                                groups.last_mut().expect("just pushed").1.push(call);
                            }
                        }
                        Entry::ToolResult {
                            call_id, result, ..
                        } => {
                            if let Some(g) = groups.last_mut() {
                                g.2.push((call_id.clone(), result.clone()));
                            }
                        }
                        _ => unreachable!("block holds only tool entries"),
                    }
                }
                for (text, calls, results) in groups {
                    // A group whose every call dangles has nothing legal to
                    // emit — drop it (the old repair pass did this after the
                    // fact; doing it here keeps construction the only source
                    // of truth).
                    if calls.is_empty() {
                        continue;
                    }
                    rebuilt = rebuilt.push(crate::ai::types::Message::Assistant {
                        content: (!text.is_empty()).then_some(text),
                        tool_calls: calls,
                    });
                    for (call_id, result) in results {
                        rebuilt = rebuilt.push(crate::ai::types::Message::Tool {
                            tool_call_id: call_id,
                            content: result,
                        });
                    }
                }
                i = end;
            }
            // A compaction marker is the context root: everything the
            // caller passed *before* it is dropped upstream (the compacted
            // region never reaches here); the summary becomes a plain user
            // turn (see compact.rs for why not System).
            Entry::Compaction { summary, .. } => {
                rebuilt = rebuilt.push(crate::ai::types::Message::User {
                    content: summary.clone(),
                });
                i += 1;
            }
            // System notices and Name markers are UI/persistence metadata:
            // neither has a protocol role.
            Entry::Error { .. } | Entry::Name { .. } | Entry::System { .. } => i += 1,
        }
    }
    rebuilt
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The byte-fidelity contract: a tool round carrying assistant text and
    /// several calls must replay as the **one** Assistant message the model
    /// sent, not N disconnected messages.
    #[test]
    fn replay_regroups_a_multi_call_message_with_its_text() {
        use crate::ai::types::{Message, ToolCall};
        let es = vec![
            Entry::User {
                content: "跑两个".into(),
            },
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "bash".into(),
                args: r#"{"command":"a"}"#.into(),
                intent: String::new(),
                // The model said something *and* called two tools.
                text: "我先跑两个命令".into(),
                first: true,
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "bash".into(),
                ok: true,
                result: "a-out".into(),
            },
            Entry::ToolRequest {
                call_id: "c2".into(),
                name: "bash".into(),
                args: r#"{"command":"b"}"#.into(),
                intent: String::new(),
                text: String::new(),
                first: false,
            },
            Entry::ToolResult {
                call_id: "c2".into(),
                name: "bash".into(),
                ok: true,
                result: "b-out".into(),
            },
            Entry::Assistant {
                content: "都好了".into(),
                usage: None,
            },
        ];
        let ctx = entries_to_context("sys", &es);
        // system, user, assistant(1 msg, 2 calls), tool, tool, assistant
        assert_eq!(
            ctx.messages.len(),
            6,
            "多调用必须合成一条: {:?}",
            ctx.messages
        );
        match &ctx.messages[2] {
            Message::Assistant {
                content,
                tool_calls,
            } => {
                assert_eq!(content.as_deref(), Some("我先跑两个命令"), "文本必须还原");
                let ids: Vec<&str> = tool_calls.iter().map(|c| c.id.as_str()).collect();
                assert_eq!(ids, vec!["c1", "c2"], "两个调用同属一条消息");
                assert!(matches!(tool_calls[0], ToolCall { .. }));
            }
            other => panic!("第 3 条应是合并后的 Assistant: {other:?}"),
        }
    }

    /// A dangling tail (call with no result) must not become a tool_calls
    /// message — the gateway rejects a call with no matching result.
    #[test]
    fn replay_drops_a_call_without_a_result() {
        use crate::ai::types::Message;
        let es = vec![
            Entry::User {
                content: "q".into(),
            },
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "bash".into(),
                args: "{}".into(),
                intent: String::new(),
                text: "想着要跑".into(),
                first: true,
            },
            // no result — the turn died here
        ];
        let ctx = entries_to_context("sys", &es);
        // system + user only: the dangling call (text and all) is dropped.
        assert_eq!(ctx.messages.len(), 2, "{:?}", ctx.messages);
        assert!(matches!(&ctx.messages[1], Message::User { .. }));
    }

    /// The user's contract: a full round (reasoning + reply + tool round)
    /// persists and reads back **byte-identical**, and the rebuilt
    /// protocol context contains the reasoning nowhere.
    #[test]
    fn full_round_with_reasoning_round_trips_byte_identical() {
        use crate::entry::Entry;
        use crate::server::turn::entries_to_context;
        use crate::store::Store;

        let dir = std::env::temp_dir().join(format!("mypi-full-round-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Store::open(&dir.join("t.db")).unwrap();
        let id = s.create_session("t", "/").unwrap();
        let round = &[
            Entry::User {
                content: "先想再答".into(),
            },
            Entry::Reasoning {
                content: "内心独白：\n1. 想一步\n2. 想两步".into(),
            },
            Entry::Assistant {
                content: "最终答案".into(),
                usage: None,
            },
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "bash".into(),
                args: r#"{"command":"echo hi"}"#.into(),
                intent: "打个招呼".into(),
                text: String::new(),
                first: true,
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "bash".into(),
                ok: true,
                result: "hi".into(),
            },
        ];
        s.append(id, round).unwrap();
        let back = s.load_entries(id).unwrap();
        assert_eq!(&back, round, "落盘→读回必须逐字节还原");

        // And the protocol rebuild skips the reasoning entirely:
        let ctx = entries_to_context("sys", &back);
        let has_reasoning_text = ctx
            .messages
            .iter()
            .any(|m| format!("{m:?}").contains("内心独白"));
        assert!(!has_reasoning_text, "推理不得进入协议上下文");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The user's hard contract, end to end: a conversation whose rounds
    /// include assistant text on tool turns, several calls in one message,
    /// reasoning, and a mid-turn break — persisted, read back, and replayed
    /// as protocol JSON — must serialize to **exactly** the bytes the model
    /// originally saw. Not "a semantically equivalent history": the same
    /// bytes, so the prefix cache hits and resume is indistinguishable.
    #[test]
    fn a_whole_conversation_replays_byte_identically() {
        use crate::ai::types::{Context as Wire, Message, ToolCall};
        use crate::entry::Entry;
        use crate::server::turn::entries_to_context;
        use crate::store::Store;

        let dir = std::env::temp_dir().join(format!("mypi-bytefit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = Store::open(&dir.join("t.db")).unwrap();
        let id = s.create_session("t", "/").unwrap();

        // The conversation as the model saw it (wire form) and the entries
        // the event stream produces for the same turn — these must agree.
        let entries = vec![
            Entry::User {
                content: "帮我看看".into(),
            },
            Entry::Reasoning {
                content: "先想想".into(),
            },
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "bash".into(),
                args: r#"{"command":"ls"}"#.into(),
                intent: "列目录".into(),
                text: "我先列个目录".into(),
                first: true,
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "bash".into(),
                ok: true,
                result: "a.txt\nb.txt".into(),
            },
            Entry::Assistant {
                content: "有两个文件".into(),
                usage: Some(Entry::usage_summary(&crate::ai::types::Usage::default())),
            },
        ];
        s.append(id, &entries).unwrap();

        // Read back from disk and replay.
        let back = s.load_entries(id).unwrap();
        assert_eq!(back, entries, "落盘→读回逐字节还原");
        let replayed = entries_to_context("sys", &back);

        // The expected wire: exactly these messages, in order.
        let expected = Wire::new()
            .push(Message::System {
                content: "sys".into(),
            })
            .push(Message::User {
                content: "帮我看看".into(),
            })
            .push(Message::Assistant {
                content: Some("我先列个目录".into()),
                tool_calls: vec![ToolCall::new("c1", "bash", r#"{"command":"ls"}"#)],
            })
            .push(Message::Tool {
                tool_call_id: "c1".into(),
                content: "a.txt\nb.txt".into(),
            })
            .push(Message::Assistant {
                content: Some("有两个文件".into()),
                tool_calls: vec![],
            });

        assert_eq!(
            serde_json::to_string(&replayed.messages).unwrap(),
            serde_json::to_string(&expected.messages).unwrap(),
            "回放必须与原始 wire 逐字节一致\n回放: {:?}\n期望: {:?}",
            replayed.messages,
            expected.messages
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn replay_starts_at_the_last_compaction_marker() {
        // The bug this guards: a resumed /compacted session used to replay
        // the *whole* stored chain, resurrecting the region the summary
        // replaced — the user paid for those tokens a second time. Replay
        // must start at the marker.
        let es = vec![
            Entry::User {
                content: "AAAA 已压缩".into(),
            },
            Entry::Assistant {
                content: "BBBB 已压缩".into(),
                usage: None,
            },
            Entry::Compaction {
                first_kept_seq: 2,
                summary: "【摘要】".into(),
            },
            Entry::User {
                content: "保留问题".into(),
            },
        ];
        let ctx = entries_to_context("sys", &es);
        let dump = format!("{:?}", ctx.messages);
        assert!(!dump.contains("AAAA"), "压缩前内容不得复活");
        assert!(!dump.contains("BBBB"), "压缩前内容不得复活");
        assert!(dump.contains("【摘要】"), "摘要必须在");
        assert!(dump.contains("保留问题"), "保留区必须在");
        // system + summary + kept user = 3
        assert_eq!(ctx.messages.len(), 3);
    }

    #[test]
    fn replay_uses_the_given_system_prompt() {
        // Resume must not silently swap the profile's prompt for the
        // built-in one.
        let es = vec![Entry::User {
            content: "问".into(),
        }];
        let ctx = entries_to_context("我的自定义提示词", &es);
        match &ctx.messages[0] {
            crate::ai::types::Message::System { content } => {
                assert_eq!(content, "我的自定义提示词");
            }
            other => panic!("首条必须是 System: {other:?}"),
        }
    }

    #[test]
    fn rebuild_handles_complete_and_dangling_tool_tails() {
        // Complete chain: request + result survive.
        let complete = vec![
            Entry::User {
                content: "q".into(),
            },
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "bash".into(),
                args: "{}".into(),
                intent: String::new(),
                text: String::new(),
                first: true,
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "bash".into(),
                ok: true,
                result: "out".into(),
            },
            Entry::Assistant {
                content: "done".into(),
                usage: None,
            },
        ];
        let ctx = entries_to_context("sys", &complete);
        assert!(matches!(
            ctx.messages[1],
            crate::ai::types::Message::User { .. }
        ));
        assert!(
            matches!(&ctx.messages[2], crate::ai::types::Message::Assistant { tool_calls, .. } if tool_calls.len() == 1)
        );
        assert!(matches!(
            ctx.messages[3],
            crate::ai::types::Message::Tool { .. }
        ));

        // Dangling request tail: dropped (never a half-open tool_calls).
        let dangling = vec![
            Entry::User {
                content: "q".into(),
            },
            Entry::ToolRequest {
                call_id: "c2".into(),
                name: "bash".into(),
                args: "{}".into(),
                intent: String::new(),
                text: String::new(),
                first: true,
            },
        ];
        let ctx = entries_to_context("sys", &dangling);
        assert_eq!(ctx.messages.len(), 2); // system + user
    }
}
