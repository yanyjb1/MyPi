//! The lap cycle: ask, run the batch, take the mailboxes, settle.

use std::sync::Arc;

use anyhow::{Result, anyhow};

use super::ask::{CompactInput, LapInput, ResultsInput, Stop, Usage};
use super::fence::Fence;
use crate::server::agent::r#loop::pen::{self, Pen};
use crate::server::agent::r#loop::session::agent::AgentRunner;
use crate::server::agent::r#loop::session::book::{Birth, Inner, Message, Retention};
use crate::server::agent::r#loop::session::child::State as ChildState;
use crate::server::agent::r#loop::session::handle::Handle;
use crate::server::agent::r#loop::session::live::Live;
use crate::server::agent::request::contract::{History, ToolResult as Reported};
use crate::server::agent::request::live::{Element, Event, Notice, NoticeLevel, Stream};
use crate::server::agent::request::message::Message as Stored;
use crate::server::agent::tools::contract::call::BatchInput;
use crate::server::agent::tools::contract::job::{JobRunner, JobSpec, Schedule};
use crate::server::agent::tools::contract::output::Effect;
use crate::server::agent::tools::contract::output::ToolResult;
use crate::server::agent::tools::contract::view::Source;
use crate::server::agent::tools::registry::roster;
use crate::server::agent::tools::subagent::{FINAL, TASK};
use crate::server::platform::time::now_stamp;
use crate::server::storage::{Ref, RequestMeta, Row};

/// Which machine a turn runs.
#[derive(Clone)]
pub enum Mode {
    Normal,
    Compact {
        retention: Option<Retention>,
        focus: Option<String>,
    },
}

pub(crate) fn run(
    inner: &Arc<Inner>,
    handle: &Arc<Handle>,
    turn: i64,
    live: &mut Live,
    mode: Mode,
    first_turn: bool,
) -> Result<Stop> {
    let transcript = handle.transcript().clone();
    let history: Arc<dyn History> = transcript.clone();
    if let Mode::Compact { retention, focus } = mode {
        let reply = match inner.ask.compact(CompactInput {
            session: handle.id,
            history: history.clone(),
            transport: live.transport(),
            system: live.system.clone(),
            tools: live.manuals.clone(),
            retention,
            focus,
            instruction: live.compact.clone(),
            stop: handle.stop.clone(),
            worker: handle.worker.clone(),
            live: inner.out.clone(),
        }) {
            Ok(reply) => reply,
            Err(e) => {
                if stopped(handle, &e) {
                    pen::turn::settle(&inner.writer, turn, Stop::User, None, handle.meta())?;
                    return Ok(Stop::User);
                }
                broken(inner, handle, turn, None, &e);
                return Err(e);
            }
        };
        let usage = reply.usage;
        handle.publish(&usage, usage.prompt + usage.completion);
        if !reply.text.is_empty() {
            pen::turn::append_summary(&inner.writer, turn, reply.text, usage)?;
        }
        pen::turn::borrow(&inner.writer, handle.id, &reply.borrows)?;
        pen::turn::settle(&inner.writer, turn, Stop::Model, None, handle.meta())?;
        transcript.reload()?;
        return Ok(Stop::Model);
    }

    let mut usage = Usage::default();
    let mut baseline = None;
    let mut pen: Option<Arc<Pen>> = None;
    let outcome = laps(
        inner,
        handle,
        turn,
        live,
        &history,
        &transcript,
        &mut usage,
        &mut baseline,
        &mut pen,
        first_turn,
    );
    match outcome {
        Ok(stop) => {
            pen::turn::settle(&inner.writer, turn, stop, baseline, handle.meta())?;
            Ok(stop)
        }
        Err(e) => {
            keep_blocks(pen, handle);
            if stopped(handle, &e) {
                pen::turn::settle(&inner.writer, turn, Stop::User, baseline, handle.meta())?;
                return Ok(Stop::User);
            }
            broken(inner, handle, turn, baseline, &e);
            Err(e)
        }
    }
}

/// One turn's laps. Every early return lands here; the caller owns the settle.
fn laps(
    inner: &Arc<Inner>,
    handle: &Arc<Handle>,
    turn: i64,
    live: &mut Live,
    history: &Arc<dyn History>,
    transcript: &Arc<super::super::session::history::Transcript>,
    usage: &mut Usage,
    baseline: &mut Option<u64>,
    pen: &mut Option<Arc<Pen>>,
    first_turn: bool,
) -> Result<Stop> {
    let child = inner.source.child(handle.id);
    let mut fence = Fence::new(inner.settings.max_tool_rounds);
    let mut first_lap = true;

    loop {
        if handle.stop.is_stopped() {
            return Ok(Stop::User);
        }
        let wakes = drain(inner, handle, turn)?;
        let appended = !wakes.is_empty();
        for text in wakes {
            transcript.append_user(Stored::User {
                prefix: None,
                content: text,
                suffix: None,
            });
        }
        if fence.hit() {
            return Ok(Stop::Fence);
        }

        let blocks = Arc::new(Pen::new(inner.writer.clone(), turn));
        *pen = Some(blocks.clone());
        let lap = inner.ask.lap(LapInput {
            session: handle.id,
            history: history.clone(),
            blocks: blocks.clone(),
            transport: live.transport(),
            system: live.system.clone(),
            tools: live.manuals.clone(),
            stop: handle.stop.clone(),
            worker: handle.worker.clone(),
            live: inner.out.clone(),
        })?;
        if first_lap && first_turn {
            *baseline = Some(lap.usage.prompt);
        }
        first_lap = false;
        *usage += lap.usage;
        handle.publish(usage, lap.usage.prompt + lap.usage.completion);

        if lap.calls.is_empty() {
            let (_, landed_blocks) = blocks.flush()?;
            transcript.append(landed_blocks)?;
            if !appended {
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
                    transcript.append_user(Stored::User {
                        prefix: None,
                        content: text.to_string(),
                        suffix: None,
                    });
                    continue;
                }
                return Ok(lap.stop);
            }
            continue;
        }

        let input = BatchInput {
            session: handle.id,
            root: live.root.clone(),
            cwd: live.cwd.clone(),
            tools: live.tools.clone(),
            calls: lap.calls.clone(),
            stop: handle.stop.clone(),
            out: inner.out.clone(),
        };
        let results = inner.batch.run(input)?;
        // Calls the stop cut off never ran: their request blocks leave no trace.
        let unanswered: std::collections::BTreeSet<String> = lap
            .calls
            .iter()
            .filter(|call| !results.iter().any(|run| run.call_id == call.call_id))
            .map(|call| call.call_id.clone())
            .collect();
        blocks.drop_calls(&unanswered);
        let reported = ResultsInput {
            session: handle.id,
            history: history.clone(),
            calls: lap.calls.clone(),
            results: results.iter().map(report).collect(),
        };
        inner.ask.results(reported, &*blocks)?;
        let (children, deletes) = effects(&blocks, &results, live)?;
        let (landed, landed_blocks) = blocks.flush()?;
        transcript.append(landed_blocks)?;
        for (at, spec) in children {
            let id = landed.ids.get(at).copied().unwrap_or(0);
            if id <= 0 {
                return Err(anyhow!("a sub-session row was not created"));
            }
            let meta = landed.ids.get(spec.meta_at).copied().unwrap_or(0);
            if meta <= 0 {
                return Err(anyhow!("a sub-session archive was not created"));
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
                    manuals: spec.manuals,
                    compact: live.compact.clone(),
                    meta: Some(meta),
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
            return Ok(Stop::User);
        }
        fence.count();
    }
}

/// Keep what a dead lap already produced: the stored turn matches what the front end showed.
fn keep_blocks(pen: Option<Arc<Pen>>, handle: &Handle) {
    let Some(pen) = pen else {
        return;
    };
    match pen.flush() {
        Ok((_, blocks)) => {
            if let Err(e) = handle.transcript().append(blocks) {
                crate::server::platform::log::error(
                    "loop",
                    format!(
                        "session {}: a dead lap's blocks were not cached: {e:#}",
                        handle.id
                    ),
                );
            }
        }
        Err(e) => crate::server::platform::log::error(
            "loop",
            format!(
                "session {}: a dead lap's blocks were not stored: {e:#}",
                handle.id
            ),
        ),
    }
}

/// A child's list: `task` is dropped silently, `final` is added because the parent never names it.
fn child_tools(tools: &[String]) -> Vec<String> {
    let mut out: Vec<String> = tools
        .iter()
        .filter(|tool| tool.as_str() != TASK.name && tool.as_str() != FINAL.name)
        .cloned()
        .collect();
    out.push(FINAL.name.to_string());
    out
}

/// One sub-session a batch asked for.
struct Spawn {
    tools: Vec<String>,
    manuals: Arc<Vec<crate::server::agent::request::message::ToolDef>>,
    cwd: std::path::PathBuf,
    start: String,
    budget: u32,
    answer: std::path::PathBuf,
    /// The row that holds the child's own request archive.
    meta_at: usize,
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
                    let meta_at = blocks.len();
                    let tools = child_tools(tools);
                    let manuals = Arc::new(roster(&tools));
                    let tools_json = serde_json::to_string(&*manuals)
                        .map_err(|e| anyhow!("a child's tool manual is not JSON: {e}"))?;
                    blocks.push(vec![
                        Row::Request {
                            meta: RequestMeta {
                                model: live.model.clone(),
                                provider: live.provider.clone(),
                                system: live.system.clone(),
                                tools_json,
                                baseline_prompt: None,
                            },
                        },
                        Row::Session {
                            started_at: now_stamp(),
                            cwd: cwd.to_string_lossy().into_owned(),
                            model: Some((live.provider.clone(), live.model.clone())),
                            name: None,
                            anchor: Some(anchor),
                            meta: Some(Ref::Batch(meta_at)),
                        },
                    ]);
                    children.push((
                        meta_at + 1,
                        Spawn {
                            tools,
                            manuals,
                            cwd: cwd.clone(),
                            start: start.clone(),
                            budget: *max_tool_calls,
                            answer: answer.clone(),
                            meta_at,
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

/// One finished call, as the request half takes it.
fn report(result: &ToolResult) -> Reported {
    Reported {
        call_id: result.call_id.clone(),
        ok: result.ok,
        result: result.result.clone(),
        reason: result.reason.clone(),
        memo: result.memo.clone(),
        spill: result.spill.clone(),
        duration_ms: result.duration_ms,
    }
}

/// The user stopped this turn: no notice, no error text, settled as interrupted.
fn stopped(handle: &Handle, why: &anyhow::Error) -> bool {
    if handle.stop.is_stopped() {
        return true;
    }
    matches!(
        why.downcast_ref::<crate::server::agent::request::retry::Failure>(),
        Some(crate::server::agent::request::retry::Failure::Cancelled)
    )
}

fn broken(
    inner: &Arc<Inner>,
    handle: &Handle,
    turn: i64,
    baseline: Option<u64>,
    why: &anyhow::Error,
) {
    let _ = inner.out.send(Event {
        session: handle.id,
        element: Element::Notice(Notice {
            level: NoticeLevel::Error,
            text: format!("the turn broke: {why:#}"),
        }),
    });
    let _ = pen::turn::settle(&inner.writer, turn, Stop::Broken, baseline, handle.meta());
}

/// Take every wake text into this turn; the texts are the ones already written as blocks.
fn drain(inner: &Arc<Inner>, handle: &Handle, turn: i64) -> Result<Vec<String>> {
    let mut out = Vec::new();
    while let Some(text) = handle.mailbox.take_wake() {
        pen::turn::append_wake(&inner.writer, turn, text.clone())?;
        let _ = inner.out.send(Event {
            session: handle.id,
            element: Element::Stream(Stream::Wake(text.clone())),
        });
        out.push(text);
    }
    Ok(out)
}
