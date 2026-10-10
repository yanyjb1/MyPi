//! One round = one request. When the model wants tools, run them all and hand the results to request.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use anyhow::Result;
use rusqlite::Connection;

use super::{Live, Outlet, ToolEvent, Turn};
use crate::server::agent::request::content::AnswerSink;
use crate::server::agent::request::message::{AssistantMessage, StopReason, Usage};
use crate::server::agent::request::{self, ToolRun};
use crate::server::agent::tools::{self, Env, ToolProgress};
use crate::server::storage::{Submit, Writer};

/// How this round ends.
pub enum Verdict {
    /// No more tools (or it was cut short). Carries the stop reason the model gave.
    Done(StopReason),
    /// Tools were requested, run, and their results filed. Carries how many ran.
    Tools(usize),
}

/// One round, as it came out.
pub struct Lap {
    /// How it ended.
    pub verdict: Verdict,
    /// Tokens used this round.
    pub usage: Usage,
}

/// Run one round. `w` writes, `r` is the caller's read connection; `rounds` is how many ran already,
/// and `fenced` means no tool manual this time.
pub fn once(
    w: &Writer,
    r: &Connection,
    live: &Live,
    env: &mut Env,
    turn: &Turn,
    out: &mut dyn Outlet,
    rounds: usize,
    fenced: bool,
) -> Result<Lap> {
    let content = super::content(live, turn.session, fenced);
    let answer = {
        let mut sink = Sink::new(out, &live.stop);
        request::start(
            w,
            r,
            &live.transport,
            &content,
            &mut sink,
            live.stop.as_ref(),
        )?
    };

    // The request record hangs on the first round only: it says what this turn carried; later rounds
    // are the same record.
    if rounds == 0 {
        w.push(Submit::Request {
            session: turn.session,
            meta: Box::new(answer.meta.clone()),
        })?;
    }

    if answer.message.tool_calls.is_empty() || answer.stop != StopReason::ToolCalls {
        return Ok(Lap {
            verdict: Verdict::Done(answer.stop),
            usage: answer.usage,
        });
    }

    let ran = calls(env, &answer.message, out);
    request::hand_results(w, turn.session, &answer.message, &ran)?;
    Ok(Lap {
        verdict: Verdict::Tools(ran.len()),
        usage: answer.usage,
    })
}

/// Run this round's tools one by one, reporting all three moments to [`Outlet`].
fn calls(env: &mut Env, reply: &AssistantMessage, out: &mut dyn Outlet) -> Vec<ToolRun> {
    let mut ran = Vec::with_capacity(reply.tool_calls.len());
    for (i, call) in reply.tool_calls.iter().enumerate() {
        out.tool_event(&ToolEvent::Start {
            call_id: call.id.clone(),
            name: call.name().to_string(),
            args: call.function.arguments.clone(),
            intent: call.intent(),
            text: if i == 0 {
                reply.content.clone()
            } else {
                String::new()
            },
            first: i == 0,
        });

        let started = Instant::now();
        let call_id = call.id.clone();
        let mut progress = |p: ToolProgress| match p {
            ToolProgress::Output(text) => out.tool_output(&call_id, &text),
        };
        let (ok, result, memo, spill) = match tools::execute(env, call, &mut progress) {
            Ok(out) => (true, out.text, out.memo, out.spill),
            Err(e) => (false, format!("tool execution failed: {e:#}"), None, None),
        };
        let duration_ms = started.elapsed().as_millis() as u64;

        out.tool_event(&ToolEvent::Finish {
            call_id: call.id.clone(),
            name: call.name().to_string(),
            ok,
            result: result.clone(),
            memo: memo.clone(),
            duration_ms,
        });
        ran.push(ToolRun {
            call_id: call.id.clone(),
            ok,
            result,
            memo,
            spill,
            duration_ms,
        });
    }
    ran
}

/// Delta outlet: wires request's two streams onto [`Outlet`].
pub struct Sink<'a> {
    out: &'a mut dyn Outlet,
    stop: &'a Arc<AtomicBool>,
}

impl<'a> Sink<'a> {
    pub fn new(out: &'a mut dyn Outlet, stop: &'a Arc<AtomicBool>) -> Self {
        Self { out, stop }
    }
}

impl AnswerSink for Sink<'_> {
    fn content(&mut self, text: &str) -> bool {
        !self.stop.load(Ordering::Relaxed) && self.out.content(text)
    }

    fn reasoning(&mut self, text: &str) -> bool {
        !self.stop.load(Ordering::Relaxed) && self.out.reasoning(text)
    }
}
