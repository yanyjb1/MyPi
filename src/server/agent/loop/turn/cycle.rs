//! The lap cycle: ask, run the batch, take the mailboxes, settle.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::{Result, anyhow};

use super::ask::{CompactInput, LapInput, ResultsInput, Stop, Usage};
use super::env;
use super::fence::Fence;
use crate::server::agent::r#loop::pen::{self, Pen};
use crate::server::agent::r#loop::session::agent::AgentRunner;
use crate::server::agent::r#loop::session::book::{Birth, Inner, Message, Retention};
use crate::server::agent::r#loop::session::child::State as ChildState;
use crate::server::agent::r#loop::session::handle::Handle;
use crate::server::agent::r#loop::session::history::Transcript;
use crate::server::agent::r#loop::session::live::Live;
use crate::server::agent::r#loop::turn::batch::ToolResult;
use crate::server::agent::request::contract::{History, ToolResult as Reported};
use crate::server::agent::tools::contract::job::{JobHost, JobRunner, JobSpec, Schedule};
use crate::server::agent::tools::contract::{Child, Effect, Source};
use crate::server::agent::tools::{Availability, roster};
use crate::server::platform::time::now_stamp;
use crate::server::storage::Row;

/// Which machine a turn runs.
#[derive(Clone)]
pub enum Mode {
    Normal,
    Compact {
        retention: Option<Retention>,
        focus: Option<String>,
    },
}

pub fn run(
    inner: &Arc<Inner>,
    handle: &Arc<Handle>,
    turn: i64,
    live: &mut Live,
    mode: Mode,
) -> Result<Stop> {
    let shape = shape(inner, handle.id);
    let child = inner.source.child(handle.id);
    if let Mode::Compact { retention, focus } = mode {
        let reply = match inner.ask.compact(CompactInput {
            session: handle.id,
            history: Arc::new(Transcript::open(&inner.writer.path())?),
            transport: live.transport(),
            system: live.system.clone(),
            tools: manuals(&live.tools, shape),
            retention,
            focus,
            instruction: live.compact.clone(),
            stop: handle.stop.clone(),
            live: inner.out.clone(),
        }) {
            Ok(reply) => reply,
            Err(e) => {
                broken(inner, handle, turn, Usage::default(), None);
                return Err(e);
            }
        };
        let mut usage = Usage::default();
        usage += reply.usage;
        if !reply.text.is_empty() {
            pen::turn::append_summary(&inner.writer, turn, reply.text)?;
        }
        pen::turn::borrow(&inner.writer, handle.id, &reply.borrows)?;
        pen::turn::settle(
            &inner.writer,
            turn,
            Stop::Model,
            usage,
            Some(reply.usage.prompt).filter(|p| *p > 0),
            handle.meta(),
        )?;
        return Ok(Stop::Model);
    }

    let mut usage = Usage::default();
    let mut fence = Fence::new(inner.settings.max_tool_rounds);
    let mut stop = Stop::Model;
    let mut first = true;
    let mut baseline = None;

    loop {
        if handle.stop.load(Ordering::Acquire) {
            stop = Stop::User;
            break;
        }
        drain(inner, handle, turn)?;
        if fence.hit() {
            stop = Stop::Fence;
            break;
        }

        let blocks = Arc::new(Pen::new(inner.writer.clone(), turn));
        let history: Arc<dyn History> = Arc::new(Transcript::open(&inner.writer.path())?);
        let lap = match inner.ask.lap(LapInput {
            session: handle.id,
            history: history.clone(),
            blocks: blocks.clone(),
            transport: live.transport(),
            system: live.system.clone(),
            tools: manuals(&live.tools, shape),
            stop: handle.stop.clone(),
            live: inner.out.clone(),
        }) {
            Ok(lap) => lap,
            Err(e) => {
                broken(inner, handle, turn, usage, baseline);
                return Err(e);
            }
        };
        if first {
            baseline = Some(lap.usage.prompt);
        }
        first = false;
        usage += lap.usage;

        if lap.calls.is_empty() {
            blocks.flush()?;
            if handle.mailbox.is_empty() {
                if let Some(text) = child.as_ref().and_then(|child| child.nag()) {
                    pen::turn::append_user(
                        &inner.writer,
                        turn,
                        Message::Submit {
                            prefix: None,
                            text: text.to_string(),
                            suffix: None,
                        },
                    )?;
                    continue;
                }
                stop = lap.stop;
                break;
            }
            continue;
        }

        let input = env::build(
            handle.id,
            turn,
            live.root.clone(),
            live.cwd.clone(),
            live.tools.clone(),
            lap.calls.clone(),
            handle.stop.clone(),
            inner.out.clone(),
        );
        let results = match inner.batch.run(input) {
            Ok(results) => results,
            Err(e) => {
                broken(inner, handle, turn, usage, baseline);
                return Err(e);
            }
        };
        let reported = ResultsInput {
            session: handle.id,
            history: history.clone(),
            calls: lap.calls.clone(),
            results: results.iter().map(report).collect(),
        };
        if let Err(e) = inner.ask.results(reported, &*blocks) {
            broken(inner, handle, turn, usage, baseline);
            return Err(e);
        }
        let (children, deletes) = effects(&blocks, &results, live)?;
        let landed = blocks.flush()?;
        for (at, spec) in children {
            let id = landed.ids.get(at).copied().unwrap_or(0);
            if id <= 0 {
                return Err(anyhow!("a sub-session row was not created"));
            }
            let jobs = inner
                .source
                .jobs(handle.id)
                .ok_or_else(|| anyhow!("session {} has no job service", handle.id))?;
            inner.source.attach_child(
                id,
                Arc::new(ChildState::child(spec.answer.clone(), spec.budget)),
            );
            let runner: Arc<dyn JobRunner> = Arc::new(AgentRunner::new(
                Arc::downgrade(inner),
                id,
                Birth {
                    session: id,
                    provider: live.provider.clone(),
                    model: live.model.clone(),
                    system: live.system.clone(),
                    tools: spec.tools,
                    compact: live.compact.clone(),
                    meta: handle.meta(),
                    cwd: Some(spec.cwd),
                },
                spec.start,
                spec.answer,
            ));
            jobs.start(JobSpec {
                runner,
                schedule: Schedule::After(std::time::Duration::ZERO),
                timeout: None,
                output: false,
                needs_slot: true,
                kind: "agent",
                wake_on_done: true,
            })?;
        }
        if !deletes.is_empty() {
            pen::effect::apply(&inner.writer, handle.id, &deletes)?;
        }
        if results.iter().any(|result| result.stop) {
            stop = Stop::User;
            break;
        }
        fence.count();
    }

    pen::turn::settle(&inner.writer, turn, stop, usage, baseline, handle.meta())?;
    Ok(stop)
}

/// One sub-session a batch asked for.
struct Spawn {
    tools: Vec<String>,
    cwd: std::path::PathBuf,
    start: String,
    budget: u32,
    answer: std::path::PathBuf,
}

/// Put every effect into the lap's commit; return what still needs acting after it lands.
fn effects(
    blocks: &Pen,
    results: &[ToolResult],
    live: &Live,
) -> Result<(Vec<(usize, Spawn)>, Vec<pen::effect::Delete>)> {
    let mut children = Vec::new();
    let mut deletes = Vec::new();
    for result in results {
        if result.effects.is_empty() {
            continue;
        }
        let anchor = blocks
            .anchor(&result.call_id)
            .ok_or_else(|| anyhow!("no result block for call {}", result.call_id))?;
        for effect in &result.effects {
            match effect {
                Effect::Child {
                    tools,
                    cwd,
                    start,
                    max_tool_calls,
                    answer,
                } => {
                    let at = blocks.len();
                    blocks.push(vec![Row::Session {
                        started_at: now_stamp(),
                        cwd: cwd.to_string_lossy().into_owned(),
                        model: Some((live.provider.clone(), live.model.clone())),
                        name: None,
                        anchor: Some(anchor),
                        meta: None,
                    }]);
                    children.push((
                        at,
                        Spawn {
                            tools: tools.clone(),
                            cwd: cwd.clone(),
                            start: start.clone(),
                            budget: *max_tool_calls,
                            answer: answer.clone(),
                        },
                    ));
                }
                other => {
                    let (rows, delete) = pen::effect::rows(anchor, other);
                    blocks.push(rows);
                    if let Some(delete) = delete {
                        deletes.push(delete);
                    }
                }
            }
        }
    }
    Ok((children, deletes))
}

/// One session's shape, read once per turn.
pub(crate) fn shape(inner: &Arc<Inner>, session: i64) -> Availability {
    Availability {
        child: inner.source.child(session).is_some(),
        jobs: inner.source.jobs(session).is_some(),
        mcp: crate::server::agent::tools::contract::Furnish::mcp(&*inner.source).is_some(),
    }
}

/// The session's names → the request's manuals, in table order.
pub(crate) fn manuals(names: &[String], shape: Availability) -> Vec<serde_json::Value> {
    roster(names, shape)
        .iter()
        .filter_map(|tool| serde_json::to_value(tool).ok())
        .collect()
}

/// One finished call, as the request half takes it.
fn report(result: &ToolResult) -> Reported {
    Reported {
        call_id: result.call_id.clone(),
        ok: result.ok,
        result: result.result.clone(),
        memo: result.memo.clone(),
        spill: result.spill.clone(),
        duration_ms: result.duration_ms,
    }
}

fn broken(inner: &Arc<Inner>, handle: &Handle, turn: i64, usage: Usage, baseline: Option<u64>) {
    let _ = pen::turn::settle(
        &inner.writer,
        turn,
        Stop::Broken,
        usage,
        baseline,
        handle.meta(),
    );
}

fn drain(inner: &Arc<Inner>, handle: &Handle, turn: i64) -> Result<()> {
    while let Some(text) = handle.mailbox.take_wake() {
        pen::turn::append_wake(&inner.writer, turn, text)?;
    }
    while let Some(msg) = handle.mailbox.take_user() {
        pen::turn::append_user(&inner.writer, turn, msg)?;
    }
    Ok(())
}
