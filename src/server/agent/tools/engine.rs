//! The front door: one batch in, one result per call out. Gates, decode, run, stamp, measure.

use std::sync::mpsc::Sender;
use std::time::Instant;

use anyhow::Result;

use super::contract::call::Batch;
use super::contract::env::{Env, Limits};
use super::contract::output::{ToolOutput, ToolResult};
use super::contract::view::Child;
use super::registry;
use super::spec::{Args, ToolProgress};
use super::subagent::FINAL;
use crate::server::agent::request::contract::{Spill, ToolCall};
use crate::server::agent::request::live::{Element, Event, ToolEvent, Truncation};
use crate::server::platform::time::now_stamp;

/// What a call that the user stopped mid-flight reports as its reason.
const INTERRUPTED: &str = "interrupted: the user stopped the turn";

/// The one implementation of [`Batch`]. Everything a run needs arrives in its [`Env`].
pub struct Engine;

impl Batch for Engine {
    fn run(
        &self,
        env: &mut Env,
        calls: &[ToolCall],
        out: Sender<Event>,
    ) -> Result<Vec<ToolResult>> {
        env.batch_len = calls.len();
        let child = env.source.child(env.session);
        let mut halted = false;
        let mut results = Vec::with_capacity(calls.len());
        for call in calls {
            // A call that never starts leaves nothing behind: no block, no result.
            if halted || env.stop.is_stopped() {
                break;
            }
            let result = self.one(
                env,
                call,
                &out,
                child.as_deref(),
                Instant::now(),
                &mut halted,
            );
            results.push(result);
        }
        Ok(results)
    }
}

impl Engine {
    fn one(
        &self,
        env: &mut Env,
        call: &ToolCall,
        out: &Sender<Event>,
        child: Option<&dyn Child>,
        started: Instant,
        halted: &mut bool,
    ) -> ToolResult {
        let Some(tool) = registry::spec(&call.name) else {
            let why = format!("unknown tool: {}", call.name);
            return self.refused(env, call, out, &why, started);
        };
        if !env.allows(tool.name) {
            let why = format!("tool `{}` is disabled by the active profile", tool.name);
            return self.refused(env, call, out, &why, started);
        }
        let charged = tool.name != FINAL.name;
        if charged && child.is_some_and(|c| c.left() == 0) {
            let why = "tool-call budget exhausted; finish with `final submit`";
            return self.refused(env, call, out, why, started);
        }
        let stated = call
            .intent
            .as_deref()
            .is_some_and(|intent| !intent.trim().is_empty());
        if !stated {
            let why = if super::spec::intent_nested(&call.args) {
                "`intent` sits beside `input`, never inside it"
            } else {
                "missing argument `intent`: 2-6 capitalized words, present participle"
            };
            return self.refused(env, call, out, why, started);
        }
        let args = match Args::fill(tool.params, &call.args) {
            Ok(args) => args,
            Err(e) => {
                let why = e.to_string();
                return self.refused(env, call, out, &why, started);
            }
        };
        self.start(env, call, out);
        // A stop that lands while this call runs is the front door's to report, not the tool's.
        let arrived_before_stop = !env.stop.is_stopped();

        let tx = out.clone();
        let id = call.call_id.clone();
        let session = env.session;
        let mut progress = |event: ToolProgress| {
            let ToolProgress::Output(text) = event;
            let _ = tx.send(Event {
                session,
                element: Element::Tool(ToolEvent::Progress {
                    call_id: id.clone(),
                    text,
                }),
            });
        };
        let mut produced = match (tool.run)(env, &args, &mut progress) {
            Ok(out) => out,
            Err(e) => {
                let text = format!("{e:#}");
                self.end(env, call, out, false, &text, None, started, None);
                return failed(call, text, started);
            }
        };
        if charged && let Some(child) = child {
            child.charge();
        }

        let ts = now_stamp();
        for effect in &mut produced.effects {
            effect.stamp(&ts);
        }
        let ToolOutput {
            text,
            memo,
            effects,
            stop,
            fail,
        } = produced;
        let reason = fail.or_else(|| {
            (arrived_before_stop && env.stop.is_stopped()).then(|| INTERRUPTED.to_string())
        });
        let ok = reason.is_none();
        let (text, spill, truncated) = measure(tool.name, env.limits, text);
        let memo = memo.map(|value| serde_json::to_string(&value).unwrap_or_default());
        self.end(
            env,
            call,
            out,
            ok,
            &text,
            memo.as_deref(),
            started,
            truncated,
        );
        if stop {
            *halted = true;
        }
        ToolResult {
            call_id: call.call_id.clone(),
            ok,
            result: text,
            reason,
            memo,
            spill,
            duration_ms: elapsed(started),
            effects,
            stop,
        }
    }

    fn start(&self, env: &Env, call: &ToolCall, out: &Sender<Event>) {
        let _ = out.send(Event {
            session: env.session,
            element: Element::Tool(ToolEvent::Start {
                call_id: call.call_id.clone(),
                name: call.name.clone(),
                args: call.args.clone(),
                intent: call.intent.clone(),
            }),
        });
    }

    /// A call refused before it runs: the live line hears it like any other result.
    fn refused(
        &self,
        env: &Env,
        call: &ToolCall,
        out: &Sender<Event>,
        why: &str,
        started: Instant,
    ) -> ToolResult {
        self.start(env, call, out);
        self.end(env, call, out, false, why, None, started, None);
        failed(call, why, started)
    }

    #[allow(clippy::too_many_arguments)]
    fn end(
        &self,
        env: &Env,
        call: &ToolCall,
        out: &Sender<Event>,
        ok: bool,
        result: &str,
        memo: Option<&str>,
        started: Instant,
        truncated: Option<Truncation>,
    ) {
        let _ = out.send(Event {
            session: env.session,
            element: Element::Tool(ToolEvent::End {
                call_id: call.call_id.clone(),
                ok,
                result: result.to_string(),
                memo: memo.map(str::to_string),
                duration_ms: elapsed(started),
                truncated,
            }),
        });
    }
}

fn elapsed(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

fn failed(call: &ToolCall, text: impl Into<String>, started: Instant) -> ToolResult {
    ToolResult {
        call_id: call.call_id.clone(),
        ok: false,
        result: text.into(),
        reason: None,
        memo: None,
        spill: None,
        duration_ms: elapsed(started),
        effects: Vec::new(),
        stop: false,
    }
}

/// Measure: too big → split in two. The full text goes to `spill`, the body keeps the visible part;
/// the two counts describe the text before it was cut.
fn measure(
    tool: &str,
    limits: Limits,
    text: String,
) -> (String, Option<Spill>, Option<Truncation>) {
    let lines = text.lines().count();
    if lines <= limits.lines && text.len() <= limits.bytes {
        return (text, None, None);
    }
    let truncated = Truncation {
        lines: lines as u64,
        bytes: text.len() as u64,
    };
    let head = visible(&text, limits);
    let spill = Spill {
        name: tool.to_string(),
        content: text,
    };
    (head, Some(spill), Some(truncated))
}

/// The visible part: whole lines, until either the line cap or the byte cap runs out. A first line
/// over the byte cap is cut at a character boundary.
fn visible(text: &str, limits: Limits) -> String {
    let mut out = String::new();
    for line in text.lines().take(limits.lines) {
        let room = limits.bytes.saturating_sub(out.len() + 1);
        if room == 0 {
            break;
        }
        if line.len() <= room {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        out.push_str(&line[..floor_boundary(line, room)]);
        break;
    }
    out.trim_end_matches('\n').to_string()
}

/// The largest character boundary within the first `bytes` bytes.
fn floor_boundary(line: &str, bytes: usize) -> usize {
    let mut end = bytes.min(line.len());
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    end
}
