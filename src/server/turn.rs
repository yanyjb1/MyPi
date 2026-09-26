//! The turn runner — one background thread per turn.
//!
//! Mirrors pi's server-side session runner: the surface (TUI or headless
//! caller) hands over a [`TurnRequest`]; the thread drives the agent
//! loop and translates its raw callbacks into protocol
//! [`SessionEvent`]s on the given channel. It knows nothing about
//! terminals.

use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};

use crate::server::agent::loop_rs::{LoopConfig, run};
use crate::server::ai::client::Client;
use crate::server::ai::types::Context as ChatContext;
use crate::server::entry::Entry;
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
    // The workspace root (where the session started): the file tools' boundary,
    // and the one thing `cd` may not leave. `cwd` migrates; this does not.
    pub workspace_root: std::path::PathBuf,
    // The model's todo list as it stands (the session's last `Entry::Todo`):
    // the `todo` tool reads and rewrites this, and the round publishes the
    // result back as an event.
    pub todo: Vec<crate::server::entry::TodoPhase>,
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
    pub artifacts: Option<crate::server::agent::artifacts::ArtifactStore>,
    // Tool-layer knobs (bash timeout, ...) from config.yaml.
    pub tools: crate::server::ai::config::ToolsConfig,
    // Which browser the web tools drive (config.yaml → `browser:`).
    pub browser: crate::server::ai::config::BrowserConfig,
    // How decoded stream text reaches subscribers (see `config::StreamMode`).
    // The wire is decoded once, here; this only picks the emission cadence.
    pub stream_mode: crate::server::ai::config::StreamMode,
}

// The system prompt as it actually entered the request, verbatim. Empty when
// the context has none: the stored header must record what was sent, never
// invent a default (a reader of the DB cannot tell "built-in prompt" from
// "no prompt" otherwise).
fn system_message(ctx: &ChatContext) -> String {
    match ctx.messages.first() {
        Some(crate::server::ai::types::Message::System { content }) => content.clone(),
        _ => String::new(),
    }
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
            workspace_root,
            todo: todo_seed,
            cwd_slot,
            history,
            cwd_trail,
            tool_filter,
            artifacts,
            tools: tools_cfg,
            browser,
            stream_mode,
        } = req;
        let send = |ev: SessionEvent| -> bool { tx.send(ev).is_ok() };
        let chat_arc = chat.clone();
        let mut tools = crate::server::agent::tools::BuiltinTools::new(cwd)
            .with_todo(todo_seed)
            .with_workspace_root(workspace_root)
            .with_limits(tools_cfg.limits())
            .with_cwd_slot(cwd_slot)
            .with_history(history, cwd_trail)
            .with_artifacts_opt(artifacts)
            .with_bash_timeout(tools_cfg.bash_timeout_secs)
            .with_interrupt(interrupt.clone())
            .with_browser(browser)
            .with_enabled(tool_filter);
        // Snapshot for this turn: the lock is held only for the clone, never during network I/O
        let mut chat = chat.lock().expect("chat 锁中毒").clone();
        // Tool manuals ship with the request — without them the model does not know the tools exist
        chat.tools = tools.definitions_for();

        // The request header goes out **before** the first request: it is what
        // this round *is* (endpoint, model, system prompt, tool manuals, token
        // ceiling), and it is everything about the request that the transcript
        // does not carry. A round that dies mid-flight still gets stored with
        // its header, which is what makes a half-finished conversation
        // reproducible byte for byte from the database alone.
        let _ = send(SessionEvent::RequestMeta {
            model: client.model().to_string(),
            protocol: client.protocol().to_string(),
            base_url: client.base_url().to_string(),
            system: system_message(&chat),
            tools_json: serde_json::to_string(&chat.tools).unwrap_or_else(|_| "[]".to_string()),
            max_tokens: cfg.max_tokens,
        });

        // Streamed text accumulates here in both modes: the error path needs it
        // too, so a dropped connection leaves the live replica holding exactly
        // what the session persists.
        //
        // 三个回调都要碰它们（文本回调往里攒、工具回调在轮到工具时把攒的
        // **交出去**），所以装在 `Rc<RefCell<_>>` 里共用一个。
        #[derive(Default)]
        struct Buffered {
            text: String,
            reasoning: String,
        }
        let buf = std::rc::Rc::new(std::cell::RefCell::new(Buffered::default()));
        let immediate = stream_mode == crate::server::ai::config::StreamMode::Immediate;
        // 交出一份（克隆，不搬空）：回合结束时调它；错误路径随后还要读剩下
        // 的文本去对齐副本，所以这里不能把缓冲搬空（工具边界上搬空，见下）。
        let flush_tail = || {
            let b = buf.borrow();
            if !b.reasoning.is_empty() {
                let _ = send(SessionEvent::ReasoningDelta(b.reasoning.clone()));
            }
            if !b.text.is_empty() {
                let _ = send(SessionEvent::Delta(b.text.clone()));
            }
        };
        // Callback returning false -> client stops reading and disconnects (a real interrupt; no wasted tokens)
        let r = run(
            &client,
            &mut chat,
            &text,
            &cfg,
            &mut tools,
            |delta| {
                buf.borrow_mut().text.push_str(delta);
                if immediate {
                    let _ = send(SessionEvent::Delta(delta.to_string()));
                }
                !interrupt.load(std::sync::atomic::Ordering::Relaxed)
            },
            |r| {
                buf.borrow_mut().reasoning.push_str(r);
                if immediate {
                    let _ = send(SessionEvent::ReasoningDelta(r.to_string()));
                }
                // Thinking counts as activity: false = interrupt, stop
                // reading (the reply is still usable content-wise).
                !interrupt.load(std::sync::atomic::Ordering::Relaxed)
            },
            |ev| {
                // buffered 模式：一轮的文本到「要调工具」就结束了，**这里**
                // 就是它的交付点。三件事都要求在这里交，而不是憋到回合结束：
                //   - 「一块一块」而不是「一个 turn 一个 turn」；
                //   - 那半句随后会作为工具条目的一部分存档，交在这里正好衔接；
                //   - 憋到最后再灌一遍会**重复**：会话在 ToolStart 时已把流式
                //     槽清空，回合末再灌就把工具调用前那半句粘进最终回复里了。
                if !immediate
                    && matches!(ev, crate::server::agent::loop_rs::ToolEvent::Start { .. })
                {
                    let (reasoning, text) = {
                        let mut b = buf.borrow_mut();
                        (
                            std::mem::take(&mut b.reasoning),
                            std::mem::take(&mut b.text),
                        )
                    };
                    if !reasoning.is_empty() {
                        let _ = send(SessionEvent::ReasoningDelta(reasoning));
                    }
                    if !text.is_empty() {
                        let _ = send(SessionEvent::Delta(text));
                    }
                }
                let _ = send(match ev {
                    crate::server::agent::loop_rs::ToolEvent::Start {
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
                    crate::server::agent::loop_rs::ToolEvent::Progress {
                        call_id,
                        progress,
                    } => SessionEvent::ToolProgress {
                        call_id,
                        chunk: match progress {
                            crate::server::agent::loop_rs::ToolProgress::Output(text) => text,
                        },
                    },
                    crate::server::agent::loop_rs::ToolEvent::Finish {
                        call_id,
                        name,
                        ok,
                        result,
                        details,
                        duration_ms,
                    } => SessionEvent::ToolFinish {
                        call_id,
                        name,
                        ok,
                        result,
                        details,
                        duration_ms,
                    },
                });
            },
        );
        // Buffered mode: hand the round over in one piece, *before* the round's
        // completion events, so subscribers observe the same order either way.
        // A round that died mid-flight delivers what it had (below), so the two
        // modes differ in emission count, never in stored text.
        if !immediate {
            flush_tail();
        }

        // 工具这一轮把清单改了就发出去（`todo` 工具写槽，这里发布——工具层不
        // 认识会话事件，会话也不认识工具）。放在回合事件**之前**：转录里先有
        // 状态，再有描述它的收尾。
        if let Some(phases) = tools.take_todo() {
            let _ = send(SessionEvent::Todo { phases });
        }

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
                // Mid-flight death (network drop, malformed stream, tool round
                // that already ran). The session persists whatever streamed, so
                // the replica must record the same thing — otherwise the next
                // request carries *less* than the database says happened, and
                // replay stops matching reality. `recorded_message` is reused so
                // the placeholder rule (empty reply) cannot drift.
                //
                // 读的是**本轮**剩下那截：前面几轮的文本已经在工具边界上
                // 交出去并被会话清槽了（见上面的交付点），`run` 也把那些
                // 轮的助手消息写进了 `chat`，再带一遍就重复了。
                let partial = crate::server::ai::types::AssistantMessage {
                    content: buf.borrow().text.clone(),
                    ..Default::default()
                };
                chat.messages.push(crate::server::agent::loop_rs::recorded_message(&partial, true));
                *chat_arc.lock().expect("chat 锁中毒") = chat;
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
    let mut rebuilt = ChatContext::new().push(crate::server::ai::types::Message::System {
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
                rebuilt = rebuilt.push(crate::server::ai::types::Message::User {
                    content: content.clone(),
                });
                i += 1;
            }
            // Reasoning never re-enters the protocol: display-only.
            Entry::Assistant { content, .. } => {
                rebuilt = rebuilt.push(crate::server::ai::types::Message::Assistant {
                    content: Some(content.clone()),
                    tool_calls: Vec::new(),
                });
                i += 1;
            }
            Entry::Reasoning { .. } => i += 1,
            // The todo list never re-enters the protocol as an entry: it is
            // re-injected as a note after the loop (see below), so a compaction
            // cannot take the model's own memo away from it.
            Entry::Todo { .. } => i += 1,
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
                    Vec<crate::server::ai::types::ToolCall>,
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
                                let call = crate::server::ai::types::ToolCall {
                                    id: call_id.clone(),
                                    kind: "function".into(),
                                    function: crate::server::ai::types::FunctionCall {
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
                    rebuilt = rebuilt.push(crate::server::ai::types::Message::Assistant {
                        content: (!text.is_empty()).then_some(text),
                        tool_calls: calls,
                    });
                    for (call_id, result) in results {
                        rebuilt = rebuilt.push(crate::server::ai::types::Message::Tool {
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
                rebuilt = rebuilt.push(crate::server::ai::types::Message::User {
                    content: summary.clone(),
                });
                i += 1;
            }
            // System notices and Name markers are UI/persistence metadata:
            // neither has a protocol role.
            Entry::Error { .. } | Entry::Name { .. } | Entry::System { .. } => i += 1,
        }
    }
    // The todo list is the model's **external memo**, so it goes into every
    // context — including one that starts at a compaction marker. Dropping it
    // there would mean the model wakes up after a compression having "forgotten"
    // the list it was working from, which is exactly what the list exists to
    // prevent. It rides as a user turn (a note the model reads, not a
    // protocol message it could have sent).
    let todo = entries
        .iter()
        .rev()
        .find_map(|e| match e {
            Entry::Todo { phases } => Some(phases.clone()),
            _ => None,
        })
        .unwrap_or_default();
    if !todo.is_empty() {
        rebuilt = rebuilt.push(crate::server::ai::types::Message::User {
            content: format!(
                "[任务清单（当前状态）]\n{}\n（这是你自己的备忘录，用 todo 工具维护）",
                crate::server::agent::todo::summary_text(&todo)
            ),
        });
    }
    rebuilt
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- 真跑一轮：假网关挡在网络上 ----
    //
    // 这一组是服务端的端到端自检：真的发 HTTP、真的解流、真的把事件交给状态机，
    // 断言的是「库里存下来的东西能复现当时发出去的字节」。

    use crate::server::agent::loop_rs::LoopConfig;
    use crate::server::test_gateway::{fake_gateway, fake_gateway_status, sse, tool_script};
    use crate::server::ai::client::{Client, RetryPolicy};
    use crate::server::ai::config::{StreamMode, ToolsConfig};
    use crate::server::ai::types::{Context as ChatContext, Message};
    use crate::server::session::SessionState;
    use crate::server::store::Store;
    use std::sync::atomic::AtomicBool;
    use std::sync::{Arc, Mutex, RwLock};

    fn run_turn(script: Vec<String>, mode: StreamMode) -> Vec<SessionEvent> {
        let (base, srv) = fake_gateway(script);
        let events = run_turn_with_base(base, "model-test", mode, None);
        // 网关线程必须收尾（抓到的请求体在这条用例里不看）
        let _ = srv.join();
        events
    }

    // 跑一轮，客户端指向给定的网关地址（重试用例要自己数请求）。
    fn run_turn_with_base(
        base: String,
        model: &str,
        mode: StreamMode,
        retry: Option<RetryPolicy>,
    ) -> Vec<SessionEvent> {
        let chat = Arc::new(Mutex::new(
            ChatContext::new().push(Message::System {
                content: "SYS".into(),
            }),
        ));
        let (tx, rx) = std::sync::mpsc::channel();
        let cwd = std::env::temp_dir();
        let client = Client::new(&base, "k", model);
        spawn_turn(
            tx,
            TurnRequest {
                client: match retry {
                    Some(p) => client.with_retry(p),
                    None => client,
                },
                chat,
                text: "hello".into(),
                cfg: LoopConfig::new(4096),
                interrupt: Arc::new(AtomicBool::new(false)),
                cwd: cwd.clone(),
                workspace_root: cwd.clone(),
                todo: Vec::new(),
                cwd_slot: Arc::new(RwLock::new(cwd)),
                history: Arc::new(Vec::new()),
                cwd_trail: Arc::new(Vec::new()),
                tool_filter: None,
                artifacts: None,
                tools: ToolsConfig::default(),
                browser: Default::default(),
                stream_mode: mode,
            },
        );
        let mut events = Vec::new();
        while let Ok(ev) = rx.recv_timeout(std::time::Duration::from_secs(10)) {
            let done = matches!(ev, SessionEvent::Done);
            events.push(ev);
            if done {
                break;
            }
        }
        events
    }

    fn deltas(events: &[SessionEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|e| match e {
                SessionEvent::Delta(d) => Some(d.clone()),
                _ => None,
            })
            .collect()
    }


    #[test]
    fn a_tool_round_stores_every_message_the_model_saw() {
        // 「进入模型上下文的消息都要落盘」：一次工具轮里，第二次请求带的是
        // system + user + assistant(带 tool_calls) + tool(结果)，这四条全部要能从
        // 库里重建，且和当时发出去的字节一致。
        let dir = std::env::temp_dir().join(format!("mypi-toolround-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("x.txt"), "文件内容\n").unwrap();

        let mut st = SessionState::new(Some(Store::open(&dir.join("t.db")).unwrap()));
        let id = st.ensure_session(&dir).unwrap();
        let _ = st.start_turn("读一下 x.txt");

        let args = r#"{"intent":"看一眼","path":"./x.txt"}"#;
        let (base, srv) = fake_gateway(tool_script(args));
        let chat = Arc::new(Mutex::new(
            ChatContext::new().push(Message::System {
                content: "SYS".into(),
            }),
        ));
        let (tx, rx) = std::sync::mpsc::channel();
        let cwd = dir.clone();
        spawn_turn(
            tx,
            TurnRequest {
                client: crate::server::ai::client::Client::new(&base, "k", "model-test"),
                chat,
                text: "读一下 x.txt".into(),
                cfg: crate::server::agent::loop_rs::LoopConfig::new(4096),
                interrupt: Arc::new(AtomicBool::new(false)),
                cwd: cwd.clone(),
                workspace_root: cwd.clone(),
                todo: Vec::new(),
                cwd_slot: Arc::new(RwLock::new(cwd)),
                history: Arc::new(Vec::new()),
                cwd_trail: Arc::new(Vec::new()),
                tool_filter: None,
                artifacts: None,
                tools: ToolsConfig::default(),
                browser: Default::default(),
                stream_mode: StreamMode::Immediate,
            },
        );
        while let Ok(ev) = rx.recv_timeout(std::time::Duration::from_secs(10)) {
            let done = matches!(ev, SessionEvent::Done);
            let _ = st.handle(ev);
            if done {
                break;
            }
        }

        let bodies = srv.join().unwrap();
        assert_eq!(bodies.len(), 2, "一次工具轮 = 两次请求");
        let live2: serde_json::Value = serde_json::from_str(&bodies[1]).unwrap();

        // 库里那条请求头：参数原样、结果原文
        let entries = st.store().unwrap().load_entries(id).unwrap();
        let req = entries
            .iter()
            .find_map(|e| match e {
                Entry::ToolRequest { args, name, .. } => Some((name.clone(), args.clone())),
                _ => None,
            })
            .expect("工具请求必须落盘");
        assert_eq!(req.0, "read");
        assert_eq!(req.1, args, "args 必须是模型原样发来的 JSON 串");
        let (ok, result) = entries
            .iter()
            .find_map(|e| match e {
                Entry::ToolResult { ok, result, .. } => Some((*ok, result.clone())),
                _ => None,
            })
            .expect("工具结果必须落盘");
        assert!(ok);
        assert!(result.contains("文件内容"), "结果应当是读到的正文：{result}");

        // 逐字节前缀：第二次请求的所有 message，都能从库里重建
        let rp = st.replay_round(id, 1).unwrap();
        let live_msgs = live2["messages"].as_array().unwrap();
        let rp_msgs = rp.messages.as_array().unwrap();
        assert_eq!(
            rp_msgs.len(),
            live_msgs.len() + 1,
            "库里比最后一次请求多一条（最终回复）"
        );
        for (i, (a, b)) in rp_msgs[..live_msgs.len()].iter().zip(live_msgs).enumerate() {
            assert_eq!(a, b, "第 {i} 条 message 不一致");
        }
        // 形状也要对：assistant 那条带 tool_calls，后面跟一条 tool
        assert_eq!(live_msgs[2]["role"], serde_json::json!("assistant"));
        assert_eq!(
            live_msgs[2]["tool_calls"][0]["function"]["arguments"],
            serde_json::json!(args)
        );
        assert_eq!(live_msgs[3]["role"], serde_json::json!("tool"));
        assert_eq!(live_msgs[3]["content"], serde_json::json!(result));
        // 两次请求的工具手册一致，且都是当时那一份
        assert_eq!(rp.body["tools"], live2["tools"]);
        assert_eq!(rp.stop_reason.as_deref(), Some("stop"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_transient_http_failure_is_retried_and_logged() {
        // 常用设定：3 次机会。网关先 500 两次再成功 —— 这一轮照常完成，而且
        // 「重试过」要能在内存日志里查到（不落盘：那只是我们自己的行为）。
        let model = "model-retry-case";
        let (base, srv) = fake_gateway_status(vec![
            (500, "boom".into()),
            (500, "boom".into()),
            (200, sse(&["好"])),
        ]);
        let events = run_turn_with_base(
            base,
            model,
            StreamMode::Immediate,
            Some(RetryPolicy {
                attempts: 3,
                base_ms: 1,
                factor: 2,
                max_ms: 4,
                jitter: false,
            }),
        );
        assert_eq!(srv.join().unwrap().len(), 3, "两次失败之后还会再发一次");
        assert_eq!(deltas(&events), vec!["好"], "重试之后照常拿到正文");
        assert!(events.iter().any(|e| matches!(e, SessionEvent::TurnDone(..))));
        assert!(
            !events.iter().any(|e| matches!(e, SessionEvent::Error(_))),
            "两次重试之后成功，不该报错"
        );
        let logged = crate::server::log::by_scope(model);
        assert_eq!(
            logged
                .iter()
                .filter(|r| r.level == crate::server::log::Level::Warn)
                .count(),
            2,
            "两次失败各记一条：{logged:?}"
        );
        assert!(logged[0].text.contains("第 1/3 次"), "{:?}", logged[0]);
        assert!(logged.iter().any(|r| r.text.contains("成功")));
    }

    #[test]
    fn a_stream_that_already_emitted_is_never_retried() {
        // 已经吐过字就不能重试：重试会把开头再放一遍。规则是「没吐过才许重试」。
        let (base, srv) = fake_gateway_status(vec![(
            200,
            "data: {\"choices\":[{\"delta\":{\"content\":\"半\"}}]}\n\ndata: {oops}\n\n"
                .into(),
        )]);
        let events = run_turn_with_base(
            base,
            "model-noretry-case",
            StreamMode::Immediate,
            Some(RetryPolicy {
                attempts: 5,
                base_ms: 1,
                factor: 2,
                max_ms: 2,
                jitter: false,
            }),
        );
        assert_eq!(srv.join().unwrap().len(), 1, "已经流出的内容不许重发");
        assert!(
            events.iter().any(|e| matches!(e, SessionEvent::Error(_))),
            "坏流必须报错结束这一轮"
        );
    }

    #[test]
    fn the_round_announces_its_request_header_before_asking() {
        // 请求头必须先于内容发出：回合半途死掉也要有头可存。
        let r = run_turn(vec![sse(&["你", "好"])], StreamMode::Immediate);
        match &r[0] {
            SessionEvent::RequestMeta {
                model,
                protocol,
                system,
                tools_json,
                max_tokens,
                base_url,
            } => {
                assert_eq!(model, "model-test");
                assert_eq!(protocol, "openai-chat-completions");
                assert_eq!(system, "SYS", "系统提示词按原文进库");
                assert!(base_url.starts_with("http://127.0.0.1:"), "{base_url}");
                assert_eq!(*max_tokens, 4096);
                let tools: Vec<serde_json::Value> = serde_json::from_str(tools_json).unwrap();
                assert!(!tools.is_empty(), "工具手册要一起进库");
            }
            other => panic!("第一个事件应当是请求头，实际是 {other:?}"),
        }
    }

    #[test]
    fn buffered_hands_the_turn_over_in_one_piece() {
        // 同一个脚本，两种模式：最终文本一致，交付次数天差地别。
        let imm = run_turn(vec![sse(&["你", "好", "呀"])], StreamMode::Immediate);
        assert_eq!(deltas(&imm), vec!["你", "好", "呀"]);
        let buf = run_turn(vec![sse(&["你", "好", "呀"])], StreamMode::Buffered);
        assert_eq!(deltas(&buf), vec!["你好呀"]);
    }

    #[test]
    fn buffered_hands_over_each_block_not_the_whole_turn() {
        // 一个回合两轮（先调工具，再收尾）。buffered 只该压住「同一轮里的
        // 半句」，不该把整回合压到最后：第一轮那半句必须在工具卡**之前**
        // 交出去。憋到最后交就不是「一块一块」而是「一个 turn 一个 turn」。
        let buf = run_turn(tool_script(r#"{"path":"x"}"#), StreamMode::Buffered);
        let text_at = buf
            .iter()
            .position(|e| matches!(e, SessionEvent::Delta(d) if d == "我先看看"))
            .unwrap_or_else(|| panic!("第一轮那半句没交出来：{buf:?}"));
        let tool_at = buf
            .iter()
            .position(|e| matches!(e, SessionEvent::ToolStart { .. }))
            .unwrap_or_else(|| panic!("工具调用没发出来：{buf:?}"));
        assert!(
            text_at < tool_at,
            "第一轮那半句压到工具之后了（= 一个 turn 一次）：{buf:?}"
        );
        // 每轮一份，不是把整回合粘成一份。
        assert_eq!(deltas(&buf), vec!["我先看看", "读完了"]);
    }

    #[test]
    fn a_todo_call_publishes_the_list_and_the_context_keeps_it() {
        // 清单是模型的**外部备忘录**：它必须 (a) 变成会话状态（resume/分支
        // 切换跟着走），(b) 进每一次上下文——包括从压缩点开始的上下文，否则
        // 模型压缩完就"忘了"自己在做什么。
        let events = run_turn(
            crate::server::test_gateway::tool_script_named(
                "todo",
                r#"{"op":"init","list":[{"phase":"阶段一","items":["甲","乙"]}]}"#,
            ),
            StreamMode::Immediate,
        );
        let phases = events
            .iter()
            .find_map(|e| match e {
                SessionEvent::Todo { phases } => Some(phases.clone()),
                _ => None,
            })
            .expect("清单改动必须发事件");
        assert_eq!(phases[0].tasks.len(), 2);

        let mut st = SessionState::new(None);
        for ev in events {
            let _ = st.handle(ev);
        }
        assert!(
            st.transcript()
                .iter()
                .any(|e| matches!(e, Entry::Todo { .. })),
            "状态要落成一条 Entry::Todo"
        );
        assert_eq!(st.current_todo().len(), 1, "最后一条就是当前清单");

        let ctx = entries_to_context("SYS", st.transcript());
        let note = ctx.messages.iter().find_map(|m| match m {
            crate::server::ai::types::Message::User { content } => {
                content.contains("任务清单").then_some(content.clone())
            }
            _ => None,
        });
        let note = note.expect("上下文里必须带当前清单");
        assert!(note.contains("甲") && note.contains("乙"), "{note}");
    }

    #[test]
    fn a_tool_result_carries_its_details_and_its_timing_into_the_event() {
        // The structured payload a front end renders from has to survive
        // loop → event → entry. Losing it here would silently degrade every
        // card back to plain text, which is exactly what this refactor removed.
        //
        // `sleep 0.15` also proves the *timing* is measured around the call
        // rather than reported as zero by a tool that never tracked time.
        let events = run_turn(
            crate::server::test_gateway::tool_script_named(
                "bash",
                r#"{"command":"sleep 0.15; echo hi"}"#,
            ),
            StreamMode::Immediate,
        );
        let (details, duration_ms) = events
            .iter()
            .find_map(|e| match e {
                SessionEvent::ToolFinish {
                    details,
                    duration_ms,
                    ..
                } => Some((details.clone(), *duration_ms)),
                _ => None,
            })
            .expect("ToolFinish 必须发出来");
        let d = details.expect("bash 必须交 details");
        assert_eq!(d["kind"], "shell");
        assert_eq!(d["exit_code"], 0, "成功命令的退出码");
        assert_eq!(d["interrupted"], false);
        assert!(
            duration_ms >= 100,
            "耗时必须是量出来的（睡了 150ms，报的是 {duration_ms}ms）"
        );
    }

    /// 交付点对了，**最终回复**才不会重复：事件流灌进会话之后，助手那条
    /// 只该是最后一轮的文本——工具调用前那半句已经作为工具条目的一部分
    /// 存档了（会话在 ToolStart 时就把流式槽清空了，回合末再灌一遍会粘上）。
    #[test]
    fn buffered_keeps_the_tool_rounds_text_out_of_the_final_reply() {
        let events = run_turn(tool_script(r#"{"path":"x"}"#), StreamMode::Buffered);
        let mut st = SessionState::new(None);
        for ev in events {
            let _ = st.handle(ev);
        }
        let assistant: Vec<&str> = st
            .transcript()
            .iter()
            .filter_map(|e| match e {
                Entry::Assistant { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(assistant, vec!["读完了"], "工具调用前那半句被粘进最终回复了");
        let tool_text: Vec<&str> = st
            .transcript()
            .iter()
            .filter_map(|e| match e {
                Entry::ToolRequest { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(tool_text, vec!["我先看看"], "那半句该在那次工具调用里");
    }

    #[test]
    fn the_stored_round_reproduces_the_live_request_prefix_byte_for_byte() {
        // 服务端的硬要求：从库里读回来，必须能重建出当时那份请求 —— 而且
        // **前缀逐字节一致**（命中缓存靠的就是这段前缀）。
        let dir = std::env::temp_dir().join(format!("mypi-prefix-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut st = SessionState::new(Some(Store::open(&dir.join("t.db")).unwrap()));
        let id = st.ensure_session(&dir).unwrap();
        let _ = st.start_turn("hello");

        let (base, srv) = fake_gateway(vec![sse(&["你", "好"])]);
        let chat = Arc::new(Mutex::new(
            ChatContext::new().push(Message::System {
                content: "SYS".into(),
            }),
        ));
        let (tx, rx) = std::sync::mpsc::channel();
        let cwd = dir.clone();
        spawn_turn(
            tx,
            TurnRequest {
                client: crate::server::ai::client::Client::new(&base, "k", "model-test"),
                chat,
                text: "hello".into(),
                cfg: crate::server::agent::loop_rs::LoopConfig::new(4096),
                interrupt: Arc::new(AtomicBool::new(false)),
                cwd: cwd.clone(),
                workspace_root: cwd.clone(),
                todo: Vec::new(),
                cwd_slot: Arc::new(RwLock::new(cwd)),
                history: Arc::new(Vec::new()),
                cwd_trail: Arc::new(Vec::new()),
                tool_filter: None,
                artifacts: None,
                tools: ToolsConfig::default(),
                browser: Default::default(),
                stream_mode: StreamMode::Immediate,
            },
        );
        while let Ok(ev) = rx.recv_timeout(std::time::Duration::from_secs(10)) {
            let done = matches!(ev, SessionEvent::Done);
            let _ = st.handle(ev);
            if done {
                break;
            }
        }

        let live: serde_json::Value =
            serde_json::from_str(&srv.join().unwrap()[0]).expect("网关收到的必须是 JSON");
        let rp = st.replay_round(id, 1).expect("库里的回合必须能复现");

        // 前缀：当时发出去的那几条 message，必须和复现出来的一模一样
        let live_msgs = live["messages"].as_array().unwrap();
        let rp_msgs = rp.messages.as_array().unwrap();
        assert_eq!(
            rp_msgs.len(),
            live_msgs.len() + 1,
            "库里比请求多一条（模型那条回复）"
        );
        for (i, (a, b)) in rp_msgs[..live_msgs.len()].iter().zip(live_msgs).enumerate() {
            assert_eq!(a, b, "第 {i} 条 message 字节不一致");
        }
        // 请求头的其余字段同样来自库
        assert_eq!(live["model"], rp.body["model"]);
        assert_eq!(live["stream"], rp.body["stream"]);
        assert_eq!(live["tools"], rp.body["tools"]);
        assert_eq!(rp.protocol, "openai-chat-completions");
        assert_eq!(rp.stop_reason.as_deref(), Some("stop"));
        // token 上限是**推导**出来的（上下文越长越小），不是把配置值原样抄回来
        assert_ne!(rp.max_tokens, 4096, "必须是推导值而不是原始上限");
        assert!(rp.max_tokens < 4096 && rp.max_tokens > 4000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_malformed_stream_keeps_the_db_and_the_replica_in_step() {
        // 流里出现坏 JSON = 中途死亡。已流出的正文要落库，**副本也必须记住同一条**，
        // 否则下一次请求带的上下文比库里少，复现就对不上了。
        let dir = std::env::temp_dir().join(format!("mypi-badstream-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut st = SessionState::new(Some(Store::open(&dir.join("t.db")).unwrap()));
        let id = st.ensure_session(&dir).unwrap();
        let _ = st.start_turn("hello");

        let bad = "data: {\"choices\":[{\"delta\":{\"content\":\"半\"}}]}\n\ndata: {oops}\n\n".to_string();
        let (base, srv) = fake_gateway(vec![bad]);
        let chat = Arc::new(Mutex::new(
            ChatContext::new().push(Message::System {
                content: "SYS".into(),
            }),
        ));
        let (tx, rx) = std::sync::mpsc::channel();
        let cwd = dir.clone();
        spawn_turn(
            tx,
            TurnRequest {
                client: crate::server::ai::client::Client::new(&base, "k", "model-test"),
                chat: chat.clone(),
                text: "hello".into(),
                cfg: crate::server::agent::loop_rs::LoopConfig::new(4096),
                interrupt: Arc::new(AtomicBool::new(false)),
                cwd: cwd.clone(),
                workspace_root: cwd.clone(),
                todo: Vec::new(),
                cwd_slot: Arc::new(RwLock::new(cwd)),
                history: Arc::new(Vec::new()),
                cwd_trail: Arc::new(Vec::new()),
                tool_filter: None,
                artifacts: None,
                tools: ToolsConfig::default(),
                browser: Default::default(),
                stream_mode: StreamMode::Immediate,
            },
        );
        let mut saw_error = false;
        while let Ok(ev) = rx.recv_timeout(std::time::Duration::from_secs(10)) {
            if matches!(ev, SessionEvent::Error(_)) {
                saw_error = true;
            }
            let done = matches!(ev, SessionEvent::Done);
            let _ = st.handle(ev);
            if done {
                break;
            }
        }
        let _ = srv.join();

        assert!(saw_error, "坏流必须报错");
        let entries = st.store().unwrap().load_entries(id).unwrap();
        match entries.last() {
            Some(Entry::Assistant { content, .. }) => assert_eq!(content, "半"),
            other => panic!("库里最后一条应当是那半截回复，实际 {other:?}"),
        }
        let rows = st.store().unwrap().rounds(id).unwrap();
        assert_eq!(rows.len(), 1, "死掉的回合也要有请求头");
        assert_eq!(rows[0].stop_reason, None, "没拿到停因就留空");
        // 副本里那条必须和库里那条对得上
        match chat.lock().unwrap().messages.last() {
            Some(Message::Assistant { content, tool_calls }) => {
                assert_eq!(content.as_deref(), Some("半"));
                assert!(tool_calls.is_empty(), "半截回复不许带工具调用");
            }
            other => panic!("副本最后一条应当是那半截回复，实际 {other:?}"),
        }
        // 而且这份上下文能原样复现
        let rp = st.replay_round(id, 1).unwrap();
        assert_eq!(rp.body["messages"][2]["content"], "半");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The byte-fidelity contract: a tool round carrying assistant text and
    /// several calls must replay as the **one** Assistant message the model
    /// sent, not N disconnected messages.
    #[test]
    fn replay_regroups_a_multi_call_message_with_its_text() {
        use crate::server::ai::types::{Message, ToolCall};
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
                details: None,
                duration_ms: 0,
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
                details: None,
                duration_ms: 0,
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
        use crate::server::ai::types::Message;
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
        use crate::server::entry::Entry;
        use crate::server::turn::entries_to_context;
        use crate::server::store::Store;

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
                details: None,
                duration_ms: 0,
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
        use crate::server::ai::types::{Context as Wire, Message, ToolCall};
        use crate::server::entry::Entry;
        use crate::server::turn::entries_to_context;
        use crate::server::store::Store;

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
                details: None,
                duration_ms: 0,
            },
            Entry::Assistant {
                content: "有两个文件".into(),
                usage: Some(Entry::usage_summary(&crate::server::ai::types::Usage::default())),
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
                first_kept_entry: 2,
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
            crate::server::ai::types::Message::System { content } => {
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
                details: None,
                duration_ms: 0,
            },
            Entry::Assistant {
                content: "done".into(),
                usage: None,
            },
        ];
        let ctx = entries_to_context("sys", &complete);
        assert!(matches!(
            ctx.messages[1],
            crate::server::ai::types::Message::User { .. }
        ));
        assert!(
            matches!(&ctx.messages[2], crate::server::ai::types::Message::Assistant { tool_calls, .. } if tool_calls.len() == 1)
        );
        assert!(matches!(
            ctx.messages[3],
            crate::server::ai::types::Message::Tool { .. }
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
