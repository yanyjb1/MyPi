//! The front door: one batch in, one result per call out. Gates, decode, run, stamp, measure.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use anyhow::Result;

use super::contract::{
    BashEnv, Batch, BatchInput, Browser, Child, Env, Export, Furnish, Limits, Paths, Source,
    ToolOutput, ToolResult,
};
use super::registry::{self, Availability};
use super::spec::{Args, ToolProgress};
use super::workspace::Workspace;
use crate::server::agent::request::contract::{Spill, ToolCall};
use crate::server::agent::request::live::{Element, Event, ToolEvent};
use crate::server::platform::time::now_stamp;

/// The one implementation of [`Batch`]. It holds what is the same for every session; `Env` is
/// rebuilt per batch.
pub struct Engine {
    limits: Limits,
    bash_timeout: Duration,
    browser: Browser,
    default_budget: u32,
    source: std::sync::Arc<dyn Source>,
    furnish: std::sync::Arc<dyn Furnish>,
    export: std::sync::Arc<dyn Export>,
    paths: Paths,
}

impl Engine {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        limits: Limits,
        bash_timeout: Duration,
        browser: Browser,
        default_budget: u32,
        source: std::sync::Arc<dyn Source>,
        furnish: std::sync::Arc<dyn Furnish>,
        export: std::sync::Arc<dyn Export>,
        paths: Paths,
    ) -> Self {
        Self {
            limits,
            bash_timeout,
            browser,
            default_budget,
            source,
            furnish,
            export,
            paths,
        }
    }

    fn env(&self, input: &BatchInput) -> Result<Env> {
        Ok(Env {
            workspace: Workspace::new(input.root.clone(), input.cwd.clone()),
            limits: self.limits,
            bash: BashEnv {
                timeout: self.bash_timeout,
                interrupt: input.stop.clone(),
                shell: crate::server::platform::shell::Shell::default(),
            },
            browser: self.browser.clone(),
            session: input.session,
            batch_len: input.calls.len(),
            tools: input.tools.iter().cloned().collect(),
            aliases: self.source.aliases(input.session)?,
            default_budget: self.default_budget,
            mcp: self.furnish.mcp(),
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
        let shape = Availability {
            child: child.as_ref().is_some_and(|c| c.is_child()),
            jobs: self.source.jobs(input.session).is_some(),
            mcp: env.mcp.is_some(),
        };
        let mut halted = false;
        let mut results = Vec::with_capacity(input.calls.len());
        for call in &input.calls {
            let started = Instant::now();
            let result = if halted || input.stop.load(Ordering::Relaxed) {
                halted = true;
                failed(call, "not run: the turn was stopped", started)
            } else {
                self.one(
                    &mut env,
                    call,
                    &input,
                    &shape,
                    child.as_deref(),
                    started,
                    &mut halted,
                )
            };
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
        shape: &Availability,
        child: Option<&dyn Child>,
        started: Instant,
        halted: &mut bool,
    ) -> ToolResult {
        let Some(tool) = registry::spec(&call.name) else {
            return failed(call, format!("unknown tool: {}", call.name), started);
        };
        if !env.allows(tool.name) {
            return failed(
                call,
                format!("tool `{}` is disabled by the active profile", tool.name),
                started,
            );
        }
        if !registry::available(tool.name, shape) {
            return failed(
                call,
                format!("`{}` is unavailable in this session", tool.name),
                started,
            );
        }
        let charged = tool.name != "final";
        if charged && child.and_then(|c| c.left()) == Some(0) {
            return failed(
                call,
                "tool-call budget exhausted; finish with `final submit`",
                started,
            );
        }
        let args = match Args::fill(tool.params, &call.args) {
            Ok(args) => args,
            Err(e) => return failed(call, e.to_string(), started),
        };
        self.start(input, call);

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
                self.end(input, call, false, &text, None, started);
                return failed(call, text, started);
            }
        };
        if charged
            && let Some(child) = child
            && let Err(e) = child.charge()
        {
            let text = format!("{e:#}");
            self.end(input, call, false, &text, None, started);
            return failed(call, text, started);
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
        } = out;
        let (text, spill) = measure(tool.name, env.limits, text);
        let memo = memo.map(|value| serde_json::to_string(&value).unwrap_or_default());
        self.end(input, call, true, &text, memo.as_deref(), started);
        if stop {
            *halted = true;
        }
        ToolResult {
            call_id: call.call_id.clone(),
            ok: true,
            result: text,
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
                text: None,
            }),
        });
    }

    fn end(
        &self,
        input: &BatchInput,
        call: &ToolCall,
        ok: bool,
        result: &str,
        memo: Option<&str>,
        started: Instant,
    ) {
        let _ = input.out.send(Event {
            session: input.session,
            element: Element::Tool(ToolEvent::End {
                call_id: call.call_id.clone(),
                ok,
                result: result.to_string(),
                memo: memo.map(str::to_string),
                duration_ms: elapsed(started),
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
        memo: None,
        spill: None,
        duration_ms: elapsed(started),
        effects: Vec::new(),
        stop: false,
    }
}

/// Measure: too big → split in two. The full text goes to `spill`, the body keeps the visible part.
fn measure(tool: &str, limits: Limits, text: String) -> (String, Option<Spill>) {
    if text.lines().count() <= limits.lines && text.len() <= limits.bytes {
        return (text, None);
    }
    let head = visible(&text, limits);
    let spill = Spill {
        name: tool.to_string(),
        content: text,
    };
    (head, Some(spill))
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
