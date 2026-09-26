//! The agent loop — mirrors pi's packages/agent/src/agent-loop.ts.
//!
//! pi's runLoop spans 160 lines: steering queues, event streams, parallel
//! tools, interruption... Here only the innermost "tool-call while loop"
//! is needed:
//!
//! ```text
//! while true:
//!     assistant = request_model(full_history)
//!     if no tools requested: break            // finish_reason != tool_calls
//!     run tools, append results to history
//!     // loop back; the model sees the results and continues
//! ```
//!
//! The shape matters: every tool request costs one more lap, and how many
//! laps to run is the model's decision, not ours.

use anyhow::Result;

use crate::server::ai::client::Client;
use crate::server::ai::types::{AssistantMessage, Context, Message, StopReason, ToolCall};

// Loop configuration.
pub struct LoopConfig {
    // Token ceiling for a single response.
    pub max_tokens: u32,
    // Maximum number of tool-call rounds.
    //
    // This ceiling must exist: models occasionally fall into
    // "call tool → read result → call it again" loops, and without a
    // brake that is a bottomless money pit. pi has similar protection.
    pub max_rounds: usize,
}

impl LoopConfig {
    // A constructor instead of a `Default` impl: `max_tokens` has no
    // sensible default (it depends on the model), and a plausible-looking
    // but wrong default is worse than none.
    pub fn new(max_tokens: u32) -> Self {
        Self {
            max_tokens,
            max_rounds: 16,
        }
    }

    /// The `max_tokens` value to **send** for this request: the user's
    /// ceiling minus the context already occupied (4 chars ≈ 1 token, a
    /// deliberately cheap estimate — real usage comes back in the
    /// response and drives billing/ctx display).
    ///
    /// Rationale: `max_tokens` is a reply *budget*, not a constant to
    /// echo verbatim. Requesting a top-of-model number for a short chat
    /// makes strict gateways (observed: local vLLM-style returns 503)
    /// reject requests that would never come close to the limit.
    pub fn effective_max_tokens(&self, ctx: &Context) -> u32 {
        let used: usize = ctx.messages.iter().map(|m| m.approx_chars()).sum();
        let used_tokens = (used / 4) as u32;
        self.max_tokens.saturating_sub(used_tokens).max(256)
    }
}

// What one tool call produced.
//
// Two halves with two audiences, and keeping them apart is the whole point:
//
// * `text` is what the **model** gets. It is the only half that ever enters a
//   request, and the only half the model is allowed to reason about.
// * `details` is what a **front end** gets: structured data the tool already
//   knows (which lines changed, the exit code, how many hits), so a renderer
//   never has to re-parse prose to find out what happened. Opaque to the core
//   (`Value`), shaped by the tool, interpreted by each UI on its own.
//
// A tool with nothing structured to say returns `ToolOutput::text(..)` and
// every front end falls back to rendering the text.
#[derive(Debug)]
pub struct ToolOutput {
    pub text: String,
    pub details: Option<serde_json::Value>,
}

impl ToolOutput {
    /// Text only — the model-facing half, no structured payload.
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            details: None,
        }
    }

    /// Text plus the UI's structured payload (see the type docs).
    pub fn with_details(text: impl Into<String>, details: serde_json::Value) -> Self {
        Self {
            text: text.into(),
            details: Some(details),
        }
    }
}

impl From<String> for ToolOutput {
    fn from(text: String) -> Self {
        Self::text(text)
    }
}

impl From<&str> for ToolOutput {
    fn from(text: &str) -> Self {
        Self::text(text)
    }
}

/// A tool's **own** event, beyond the start/end pair every call gets.
///
/// Tools report *while they run* — that is the whole point: a command that
/// prints for thirty seconds must not look like a hang, and a `wait`-style tool
/// must be able to say what it is waiting for. The loop forwards these verbatim
/// and never interprets them; a tool that emits nothing is not broken, it just
/// has nothing to say between start and end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolProgress {
    /// More output was produced.
    ///
    /// **Incremental** on purpose: the front end appends it to a bounded buffer
    /// and the session owns that buffer, so a long-running command costs one
    /// small message per tick instead of a fresh copy of everything it has ever
    /// printed.
    Output(String),
}

// Tool executor: the loop hands model-requested calls over to it.
//
// A trait rather than a concrete type so the loop **never knows** which
// tools exist. Adding a tool = implement this trait and register it; the
// loop body does not change. (Corresponds to pi's AgentTool; pi's
// registry supports runtime registration, ours is static.)
pub trait ToolExecutor {
    // Run one tool call, returning the text for the model plus whatever
    // structured details the UI should render from.
    //
    // `Err` means the execution failed. Failure is **not** a loop
    // failure: the error is wrapped as a tool result and returned to the
    // model, which decides how to recover (retry with new arguments,
    // switch tools, or give up). Far more useful than breaking the loop.
    ///
    /// `progress` is the tool's channel to the user *while it runs* (see
    /// [`ToolProgress`]). A tool that finishes instantly simply never calls it;
    /// one that blocks for a minute should call it early and often.
    fn execute(
        &mut self,
        call: &ToolCall,
        progress: &mut dyn FnMut(ToolProgress),
    ) -> Result<ToolOutput>;
}

// An executor with no tools.
//
// Placeholder for sessions that have not attached tools yet. When the
// model does request a call it neither panics nor silently drops it —
// it returns an explicit error. Silent drops are the worst outcome: the
// model believes the tool ran, keeps making things up, and the user
// watches a pile of hallucinations.
pub struct NoTools;

impl ToolExecutor for NoTools {
    fn execute(
        &mut self,
        call: &ToolCall,
        _progress: &mut dyn FnMut(ToolProgress),
    ) -> Result<ToolOutput> {
        anyhow::bail!("no tools registered, cannot execute `{}`", call.name())
    }
}

// Result of one turn.
pub struct TurnOutcome {
    // The final assistant reply (content).
    pub message: AssistantMessage,
    // How many tool calls the model placed this turn (across all rounds).
    pub tool_calls_made: usize,
    // Whether the `max_rounds` brake was hit.
    pub hit_round_limit: bool,
}

// Run one full turn: append the user message to the context and keep
// going until the model stops requesting tools.
//
// `on_delta` passes through to the client; the frontend uses it for
// typewriter output. Tool events go upstream while tools execute —
// the UI renders tool cards from them.
//
// Living in the loop layer rather than the app layer: tools are the
// loop's responsibility, and only the loop knows which tool is running.
#[derive(Debug, Clone)]
pub enum ToolEvent {
    // A call is starting (the model named the tool).
    //
    // `args` is the raw JSON argument string (the card renders it); `intent`
    // is the model's own one-liner about what it is doing, surfaced while the
    // tool blocks the conversation.
    Start {
        call_id: String,
        name: String,
        args: String,
        intent: String,
        // The assistant message's text, carried by its opening call only
        // (see `Entry::ToolRequest`). Empty for a pure tool round.
        text: String,
        // True on the message's first call: lets replay regroup a run of
        // calls into the single Assistant message the model actually sent.
        first: bool,
    },
    // The tool said something while it was running (see [`ToolProgress`]).
    // Forwarded as-is: the loop has no opinion about what a tool wants to say.
    Progress {
        call_id: String,
        progress: ToolProgress,
    },
    // Execution finished.
    //
    // `result` is the model-facing text; `details` is the tool's structured
    // payload for front ends (see [`ToolOutput`]); `duration_ms` is measured
    // here, not by the tool, so every tool reports the same quantity the same
    // way — a tool that forgets cannot make the number disappear.
    Finish {
        call_id: String,
        name: String,
        ok: bool,
        result: String,
        details: Option<serde_json::Value>,
        duration_ms: u64,
    },
}

// The model's stated intent for a call (`arguments.intent`). Empty when it
// omitted the field — the UI then falls back to its generic label.
fn extract_intent(call: &ToolCall) -> String {
    call.function
        .arguments_json()
        .ok()
        .and_then(|v| v.get("intent").and_then(|i| i.as_str()).map(String::from))
        .unwrap_or_default()
}

// The reply as it goes into the context — built so that it matches, byte for
// byte, the entry the session will persist for the same reply.
//
// Two deliberate differences from `AssistantMessage::to_message`:
//  * an empty reply becomes `entry::EMPTY_REPLY` (the session's placeholder),
//    not `content: null`;
//  * `drop_calls` strips the tool calls, for replies whose calls are **not
//    executed** (the round brake). A `tool_calls` message with no matching
//    `tool` result is rejected outright by strict gateways, and the persisted
//    entries never carried those calls either — so keeping them would both
//    break the next request and desynchronize live context from replay.
pub(crate) fn recorded_message(m: &AssistantMessage, drop_calls: bool) -> Message {
    Message::Assistant {
        content: Some(if m.content.is_empty() {
            crate::server::entry::EMPTY_REPLY.to_string()
        } else {
            m.content.clone()
        }),
        tool_calls: if drop_calls {
            Vec::new()
        } else {
            m.tool_calls.clone()
        },
    }
}
// Run one full turn (see [`ToolEvent`] for the event contract).
//
// More than 7 parameters is deliberate: the three callbacks (content /
// reasoning / tool events) are three distinct data streams; merging them
// into one enum callback forces every call site to match-and-unwrap.
#[allow(clippy::too_many_arguments)]
pub fn run(
    client: &Client,
    ctx: &mut Context,
    user_input: &str,
    cfg: &LoopConfig,
    tools: &mut dyn ToolExecutor,
    mut on_delta: impl FnMut(&str) -> bool,
    mut on_reasoning: impl FnMut(&str) -> bool,
    mut on_tool: impl FnMut(ToolEvent),
) -> Result<TurnOutcome> {
    ctx.messages.push(Message::User {
        content: user_input.to_string(),
    });

    let mut tool_calls_made = 0usize;
    let mut rounds = 0usize;

    loop {
        let assistant = client.stream(
            ctx,
            cfg.effective_max_tokens(ctx),
            &mut on_delta,
            &mut on_reasoning,
        )?;

        // No tools requested, or interrupted/aborted — turn over.
        //
        // `Interrupted` still finishes cleanly: the user pressed Esc and
        // the content received so far is usable; do not spend it on tool
        // requests.
        if !assistant.has_tool_calls() || assistant.stop_reason != StopReason::ToolCalls {
            // Record the reply in the context **before** returning.
            //
            // The reply is a real conversation message: the next turn's
            // request must contain it (the model has to see what it said
            // itself, and the prefix cache keys on it), and `collect_turn`
            // projects the persisted entries out of this very context — so
            // without this push, model replies were never persisted at all
            // (the DB held user/tool rows and nothing the model said).
            ctx.messages.push(recorded_message(&assistant, false));
            return Ok(TurnOutcome {
                message: assistant,
                tool_calls_made,
                hit_round_limit: false,
            });
        }

        // ---- The model asked for tools: record the request first ----
        //
        // Not optional. The history must contain the "model requested a
        // tool" message so the following tool result has something to
        // pair with; without it gateways reject with "tool_call_id has no
        // matching call".
        ctx.messages.push(assistant.to_message());

        // ---- Execute one by one, appending results immediately ----
        // One assistant message, N calls: the message's text rides its first
        // call, and `first` marks that boundary so replay can rebuild the
        // exact wire shape instead of N disconnected messages.
        let round_text = assistant.content.clone();
        for (i, call) in assistant.tool_calls.iter().enumerate() {
            on_tool(ToolEvent::Start {
                call_id: call.id.clone(),
                name: call.name().to_string(),
                args: call.function.arguments.clone(),
                intent: extract_intent(call),
                text: if i == 0 {
                    round_text.clone()
                } else {
                    String::new()
                },
                first: i == 0,
            });
            let started = std::time::Instant::now();
            // The sink borrows `on_tool` only for the duration of the call, so
            // the loop can keep using it afterwards.
            let mut report = |p: ToolProgress| {
                on_tool(ToolEvent::Progress {
                    call_id: call.id.clone(),
                    progress: p,
                });
            };
            let (ok, content, details) = match tools.execute(call, &mut report) {
                Ok(out) => (true, out.text, out.details),
                // Failed executions also go back to the model (see the
                // trait docs), only the text differs
                Err(e) => (false, format!("tool execution failed: {e:#}"), None),
            };
            on_tool(ToolEvent::Finish {
                call_id: call.id.clone(),
                name: call.name().to_string(),
                ok,
                result: content.clone(),
                details,
                duration_ms: started.elapsed().as_millis() as u64,
            });
            ctx.messages.push(Message::Tool {
                tool_call_id: call.id.clone(),
                content,
            });
        }
        tool_calls_made += assistant.tool_calls.len();

        rounds += 1;
        if rounds >= cfg.max_rounds {
            // Brake: emit the last round's result and tell upstream the
            // ceiling was hit. The UI must surface this explicitly —
            // otherwise the user just sees a reply that mysteriously
            // stops mid-thought.
            //
            // This reply's calls are **never executed** (the brake exists
            // precisely because the model keeps asking), so they are stripped
            // from the recorded message: a `tool_calls` message without its
            // `tool` results is a protocol violation that would make the *next*
            // turn fail on strict gateways.
            let last = client.stream(
                ctx,
                cfg.effective_max_tokens(ctx),
                &mut on_delta,
                &mut on_reasoning,
            )?;
            ctx.messages.push(recorded_message(&last, true));
            return Ok(TurnOutcome {
                message: last,
                tool_calls_made,
                hit_round_limit: true,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::ai::types::{ToolCall, ToolDef};
    use serde_json::json;

    // Fake executor that records received calls and returns canned
    // results — verifies the loop's shape **without any network**.
    struct FakeTools {
        seen: Vec<String>,
        reply: Result<String, String>,
    }

    impl FakeTools {
        fn new(reply: Result<String, String>) -> Self {
            Self {
                seen: Vec::new(),
                reply,
            }
        }
    }

    impl ToolExecutor for FakeTools {
        fn execute(
            &mut self,
            call: &ToolCall,
            _progress: &mut dyn FnMut(ToolProgress),
        ) -> Result<ToolOutput> {
            self.seen.push(call.name().to_string());
            match &self.reply {
                Ok(s) => Ok(ToolOutput::text(s.clone())),
                Err(e) => anyhow::bail!("{e}"),
            }
        }
    }

    #[test]
    fn no_tools_reports_a_clear_error_instead_of_silently_dropping() {
        // The behavior most worth pinning here: model asks for a tool,
        // system has none — pretending nothing happened is the only
        // unacceptable outcome.
        let mut exec = NoTools;
        let err = exec
            .execute(&ToolCall::new("c1", "read_file", "{}"), &mut |_| {})
            .unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("read_file"), "错误里要点出工具名: {msg}");
        assert!(msg.contains("no tools registered"), "错误要说清原因: {msg}");
    }

    #[test]
    fn assistant_message_with_tool_calls_round_trips_into_history() {
        // The loop must use to_message() — not hand-rolling — when
        // recording the request into history; this test pins the exact
        // shape of that message.
        let assistant = AssistantMessage {
            content: String::new(),
            tool_calls: vec![ToolCall::new(
                "c1",
                "read_file",
                json!({"path": "a"}).to_string(),
            )],
            stop_reason: StopReason::ToolCalls,
            ..Default::default()
        };
        assert!(assistant.has_tool_calls());

        let msg = assistant.to_message();
        assert_eq!(
            msg,
            Message::Assistant {
                content: None,
                tool_calls: vec![ToolCall::new(
                    "c1",
                    "read_file",
                    json!({"path": "a"}).to_string()
                )],
            }
        );
    }

    #[test]
    fn loop_config_has_a_round_brake() {
        // A loop without a brake is a money pit; the floor requirement
        // is "it can actually stop"
        let cfg = LoopConfig::new(4096);
        assert_eq!(cfg.max_tokens, 4096);
        assert!(cfg.max_rounds > 0 && cfg.max_rounds < 1000);
    }

    #[test]
    fn fake_executor_records_and_can_fail() {
        // Guard the test double itself, so the loop tests below do not
        // build on a broken fake
        let mut ok = FakeTools::new(Ok("result".into()));
        assert_eq!(
            ok.execute(&ToolCall::new("c", "f", "{}"), &mut |_| {}).unwrap().text,
            "result"
        );
        assert_eq!(ok.seen, vec!["f"]);

        let mut bad = FakeTools::new(Err("炸了".into()));
        assert!(bad.execute(&ToolCall::new("c", "f", "{}"), &mut |_| {}).is_err());
    }

    #[test]
    fn tool_def_can_ride_along_in_context() {
        // Tool definitions travel with the Context; stream's signature
        // is untouched
        let ctx = Context::new().tool(ToolDef::function("read_file", "读文件", json!({})));
        assert_eq!(ctx.tools.len(), 1);
        assert_eq!(ctx.tools[0].function.name, "read_file");
    }

    #[test]
    fn unexecuted_calls_are_stripped_from_the_recorded_reply() {
        // The round brake: the model asked for a tool, the call was never
        // executed. Recording it anyway puts a `tool_calls` message with no
        // matching `tool` result into the history — the next request is then
        // rejected outright by strict gateways.
        let m = AssistantMessage {
            content: "还得再查".into(),
            tool_calls: vec![ToolCall::new("c9", "bash", "{}")],
            stop_reason: StopReason::ToolCalls,
            ..Default::default()
        };
        match recorded_message(&m, true) {
            Message::Assistant {
                content,
                tool_calls,
            } => {
                assert_eq!(content.as_deref(), Some("还得再查"), "文本必须保留");
                assert!(tool_calls.is_empty(), "未执行的调用不得进上下文");
            }
            other => panic!("{other:?}"),
        }
        // An executed round keeps its calls.
        match recorded_message(&m, false) {
            Message::Assistant { tool_calls, .. } => assert_eq!(tool_calls.len(), 1),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_empty_reply_is_recorded_as_the_persisted_placeholder() {
        // Byte fidelity: the session stores `EMPTY_REPLY` for a reply that
        // arrived empty, so the live context must carry the same string —
        // otherwise a restart replays different bytes than the request that
        // produced them (cold prefix cache, and the model sees a history it
        // never sent).
        let m = AssistantMessage::default();
        match recorded_message(&m, false) {
            Message::Assistant { content, .. } => assert_eq!(
                content.as_deref(),
                Some(crate::server::entry::EMPTY_REPLY),
                "空回复必须与落盘内容一致"
            ),
            other => panic!("{other:?}"),
        }
    }
}
