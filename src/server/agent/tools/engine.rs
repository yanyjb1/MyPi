//! The front door: one batch in, one result per call out. Gates, decode, run, stamp, measure.

use std::sync::mpsc::Sender;
use std::time::Instant;

use anyhow::Result;

use super::contract::call::Batch;
use super::contract::env::{Env, Limits};
use super::contract::halt::INTERRUPTED;
use super::contract::output::{ToolOutput, ToolResult};
use super::contract::view::Child;
use super::fs::{fault, grep, read};
use super::registry;
use super::spec::{Args, ToolProgress};
use super::subagent::FINAL;
use crate::platform::time::now_stamp;
use crate::server::agent::request::contract::{ItemOk, Spill, ToolCall};
use crate::server::agent::request::live::{Element, Event, ToolEvent, Truncation};

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
        env.batch = calls.to_vec();
        env.handed.clear();
        env.shells.clear();
        let child = env.source.child(env.session);
        let mut halted = false;
        let mut results = Vec::with_capacity(calls.len());
        for call in calls {
            // A tool's own stop (`final submit`) leaves the calls after it unstarted and unnamed.
            if halted {
                break;
            }
            // The user's stop is the other way round: a call that had already been asked for has a
            // result whether or not it ever ran.
            if env.stop.is_stopped() {
                results.push(interrupted(call));
                continue;
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

        // The batch's one check: every written answer is finished here, and the crate's report
        // rides on the last written answer.
        fault::settle(env, &mut results);
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
        env.current = call.call_id.clone();
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
            return self.refused(
                env,
                call,
                out,
                "missing argument `intent`: 2-6 capitalized words, present participle",
                started,
            );
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
                // A tool killed by the stop states what its death means, not only the symptom.
                let reason =
                    (arrived_before_stop && env.stop.is_stopped()).then(|| INTERRUPTED.to_string());
                let text = format!("{e:#}");
                let items = outcomes(Vec::new(), reason.as_deref(), &text);
                self.end(env, call, out, &items, &text, None, started, None);
                return failed(call, items, text, started, reason);
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
            items,
            images,
            memo,
            effects,
            stop,
            fail,
        } = produced;
        let reason = fail.or_else(|| {
            (arrived_before_stop && env.stop.is_stopped()).then(|| INTERRUPTED.to_string())
        });
        // `read` and `grep` bound their own text, so the front door never spills them.
        let own = tool.name == read::TOOL.name || tool.name == grep::TOOL.name;
        let (text, spill, truncated) = if own {
            (text, None, None)
        } else {
            measure(tool.name, env.limits, text)
        };
        let items = outcomes(items, reason.as_deref(), &text);
        let memo = memo.map(|value| serde_json::to_string(&value).unwrap_or_default());
        self.end(
            env,
            call,
            out,
            &items,
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
            items,
            result: text,
            images,
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
        let items = vec![ItemOk::failed(why)];
        self.end(env, call, out, &items, why, None, started, None);
        failed(call, items, why, started, None)
    }

    #[allow(clippy::too_many_arguments)]
    fn end(
        &self,
        env: &Env,
        call: &ToolCall,
        out: &Sender<Event>,
        items: &[ItemOk],
        result: &str,
        memo: Option<&str>,
        started: Instant,
        truncated: Option<Truncation>,
    ) {
        let _ = out.send(Event {
            session: env.session,
            element: Element::Tool(ToolEvent::End {
                call_id: call.call_id.clone(),
                items: items.to_vec(),
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

/// A call the user's stop kept from running: it still answers, so the batch keeps its shape.
fn interrupted(call: &ToolCall) -> ToolResult {
    let text = format!("the call never ran: {INTERRUPTED}");
    let items = vec![ItemOk::failed(text.clone())];
    failed(call, items, text, Instant::now(), None)
}
/// One finished call the loop reports; images stay empty on every failure path.
fn failed(
    call: &ToolCall,
    items: Vec<ItemOk>,
    text: impl Into<String>,
    started: Instant,
    reason: Option<String>,
) -> ToolResult {
    ToolResult {
        call_id: call.call_id.clone(),
        items,
        result: text.into(),
        images: Vec::new(),
        reason,
        memo: None,
        spill: None,
        duration_ms: elapsed(started),
        effects: Vec::new(),
        stop: false,
    }
}

/// The outcomes a result carries: the tool's own items, or the one answer a single-item tool
/// gives. A call cut short states its reason in that item's text, where the front end draws it.
fn outcomes(items: Vec<ItemOk>, reason: Option<&str>, text: &str) -> Vec<ItemOk> {
    if !items.is_empty() {
        return items;
    }
    let Some(reason) = reason else {
        return vec![ItemOk::ok()];
    };
    vec![ItemOk::failed(format!("{text}\n[{reason}]"))]
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
