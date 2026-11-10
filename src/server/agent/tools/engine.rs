//! The front door: one batch in, one result per call out. Gates, decode, run, stamp, measure.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use anyhow::Result;

use super::contract::call::{Batch, BatchInput};
use super::contract::env::{Aliases, BashEnv, Env, Limits, Paths};
use super::contract::output::{ToolOutput, ToolResult};
use super::contract::view::{Child, Export, Source};
use super::registry;
use super::spec::{Args, ToolProgress};
use super::subagent::FINAL;
use super::workspace::Workspace;
use crate::server::agent::request::contract::{Spill, ToolCall};
use crate::server::agent::request::live::{Element, Event, ToolEvent, Truncation};
use crate::server::platform::time::now_stamp;

/// What a call that the user stopped mid-flight reports as its reason.
const INTERRUPTED: &str = "interrupted: the user stopped the turn";

/// The one implementation of [`Batch`]. It holds what is the same for every session; `Env` is
/// rebuilt per batch.
pub struct Engine {
    limits: Limits,
    bash_timeout: Duration,
    default_budget: u32,
    source: std::sync::Arc<dyn Source>,
    export: std::sync::Arc<dyn Export>,
    aliases: BTreeMap<String, String>,
    paths: Paths,
}

impl Engine {
    pub fn new(
        limits: Limits,
        bash_timeout: Duration,
        default_budget: u32,
        source: std::sync::Arc<dyn Source>,
        export: std::sync::Arc<dyn Export>,
        aliases: BTreeMap<String, String>,
        paths: Paths,
    ) -> Self {
        Self {
            limits,
            bash_timeout,
            default_budget,
            source,
            export,
            aliases,
            paths,
        }
    }

    fn env(&self, input: &BatchInput) -> Result<Env> {
        // Aliases are a database read: only a batch that reaches the shell pays for it.
        let reads_aliases = input
            .calls
            .iter()
            .any(|call| matches!(call.name.as_str(), "bash" | "alias" | "kairos"));
        let session_aliases = if reads_aliases {
            self.source.aliases(input.session)?
        } else {
            BTreeMap::new()
        };
        Ok(Env {
            workspace: Workspace::new(input.root.clone(), input.cwd.clone()),
            limits: self.limits,
            bash: BashEnv {
                timeout: self.bash_timeout,
                shell: crate::server::platform::shell::Shell::default(),
            },
            stop: input.stop.clone(),
            session: input.session,
            batch_len: input.calls.len(),
            tools: input.tools.iter().cloned().collect(),
            aliases: Aliases::new(self.aliases.clone(), session_aliases),
            default_budget: self.default_budget,
            export: self.export.clone(),
            paths: self.paths.clone(),
            source: self.source.clone(),
        })
    }
}

impl Batch for Engine {
    fn run(&self, input: BatchInput) -> Result<Vec<ToolResult>> {
        let mut env = self.env(&input)?;
        let child = self.source.child(input.session);
        let mut halted = false;
        let mut results = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            // A call that never starts leaves nothing behind: no block, no result.
            if halted || input.stop.is_stopped() {
                break;
            }
            let result = self.one(
                &mut env,
                call,
                &input,
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
        input: &BatchInput,
        child: Option<&dyn Child>,
        started: Instant,
        halted: &mut bool,
    ) -> ToolResult {
        let Some(tool) = registry::spec(&call.name) else {
            let why = format!("unknown tool: {}", call.name);
            return self.refused(input, call, &why, started);
        };
        if !env.allows(tool.name) {
            let why = format!("tool `{}` is disabled by the active profile", tool.name);
            return self.refused(input, call, &why, started);
        }
        let charged = tool.name != FINAL.name;
        if charged && child.is_some_and(|c| c.left() == 0) {
            let why = "tool-call budget exhausted; finish with `final submit`";
            return self.refused(input, call, why, started);
        }
        let stated = call
            .intent
            .as_deref()
            .is_some_and(|intent| !intent.trim().is_empty());
        if !stated {
            let why = "missing argument `intent`: 2-6 capitalized words, present participle";
            return self.refused(input, call, why, started);
        }
        let args = match Args::fill(tool.params, &call.args) {
            Ok(args) => args,
            Err(e) => {
                let why = e.to_string();
                return self.refused(input, call, &why, started);
            }
        };
        self.start(input, call);
        // A stop that lands while this call runs is the front door's to report, not the tool's.
        let arrived_before_stop = !env.stop.is_stopped();

        let tx = input.out.clone();
        let id = call.call_id.clone();
        let mut progress = |event: ToolProgress| {
            let ToolProgress::Output(text) = event;
            let _ = tx.send(Event {
                session: input.session,
                element: Element::Tool(ToolEvent::Progress {
                    call_id: id.clone(),
                    text,
                }),
            });
        };
        let mut out = match (tool.run)(env, &args, &mut progress) {
            Ok(out) => out,
            Err(e) => {
                let text = format!("{e:#}");
                self.end(input, call, false, &text, None, started, None);
                return failed(call, text, started);
            }
        };
        if charged && let Some(child) = child {
            child.charge();
        }

        let ts = now_stamp();
        for effect in &mut out.effects {
            effect.stamp(&ts);
        }
        let ToolOutput {
            text,
            memo,
            effects,
            stop,
            fail,
        } = out;
        let reason = fail.or_else(|| {
            (arrived_before_stop && env.stop.is_stopped()).then(|| INTERRUPTED.to_string())
        });
        let ok = reason.is_none();
        let (text, spill, truncated) = measure(tool.name, env.limits, text);
        let memo = memo.map(|value| serde_json::to_string(&value).unwrap_or_default());
        self.end(input, call, ok, &text, memo.as_deref(), started, truncated);
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

    fn start(&self, input: &BatchInput, call: &ToolCall) {
        let _ = input.out.send(Event {
            session: input.session,
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
        input: &BatchInput,
        call: &ToolCall,
        why: &str,
        started: Instant,
    ) -> ToolResult {
        self.start(input, call);
        self.end(input, call, false, why, None, started, None);
        failed(call, why, started)
    }

    fn end(
        &self,
        input: &BatchInput,
        call: &ToolCall,
        ok: bool,
        result: &str,
        memo: Option<&str>,
        started: Instant,
        truncated: Option<Truncation>,
    ) {
        let _ = input.out.send(Event {
            session: input.session,
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
