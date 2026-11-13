//! The lap cycle: ask, run the batch, take the mailboxes, settle.

use std::sync::Arc;

use anyhow::{Result, anyhow};

use super::ask::{CompactInput, LapInput, ResultsInput, Stop, Usage};
use super::fence::Fence;
use crate::server::agent::r#loop::pen::{self, Pen};
use crate::server::agent::r#loop::session::agent::AgentRunner;
use crate::server::agent::r#loop::session::book::{Birth, Inner, Retention};
use crate::server::agent::r#loop::session::child::State as ChildState;
use crate::server::agent::r#loop::session::handle::Handle;
use crate::server::agent::request::contract::{History, ToolResult as Reported};
use crate::server::agent::request::live::{Element, Event, Notice, NoticeLevel, Stream};
use crate::server::agent::request::message::Message as Stored;
use crate::server::agent::tools::contract::job::{JobRunner, JobSpec, Schedule};
use crate::server::agent::tools::contract::output::Effect;
use crate::server::agent::tools::contract::output::ToolResult;
use crate::server::agent::tools::contract::view::Source;
use crate::server::agent::tools::registry::roster;
use crate::server::agent::tools::subagent::{FINAL, TASK};
use crate::server::platform::time::now_stamp;
use crate::server::storage::{Ref, RequestMeta, Row};

/// The label a compaction's failure carries; the round itself is not the conversation's.
const COMPACTION_FAILED: &str = "COMPACTION FAILED";

/// Which machine a turn runs.
#[derive(Clone)]
pub enum Mode {
    Normal,
    Compact {
        retention: Option<Retention>,
        /// Rendered when the round opened; the same text the round's own block holds.
        instruction: String,
    },
}

pub(crate) fn run(
    inner: &Arc<Inner>,
    handle: &Arc<Handle>,
    turn: i64,
    mode: Mode,
    first_turn: bool,
) -> Result<Stop> {
    let transcript = handle.transcript().clone();
    let history: Arc<dyn History> = transcript.clone();
    if let Mode::Compact {
        retention,
        instruction,
    } = mode
    {
        let config = handle.config();
        let reply = match inner.ask.compact(CompactInput {
            session: handle.id,
            turn,
            history: history.clone(),
            transport: config.transport(),
            system: config.system.clone(),
            tools: config.manuals.clone(),
            tools_json: config.tools_json.clone(),
            retention,
            instruction: instruction.clone(),
            stop: handle.stop.clone(),
            worker: handle.worker.clone(),
            live: inner.out.clone(),
        }) {
            Ok(reply) => reply,
            Err(e) => {
                if stopped(handle, &e) {
                    settle(inner, handle, turn, Stop::User, None)?;
                    return Ok(Stop::User);
                }
                failed(inner, handle, turn, None, &e, COMPACTION_FAILED);
                return Err(e);
            }
        };
        let usage = reply.usage;
        handle.publish(&usage, usage.prompt + usage.completion);
        // One submission: the summary, the rounds it kept, and the mark that makes this round the
        // history's start. A compaction that broke above leaves the list exactly as it found it.
        pen::turn::land_compaction(
            &inner.writer,
            handle.id,
            turn,
            instruction,
            reply.text,
            usage,
            &reply.borrows,
        )?;
        settle(inner, handle, turn, Stop::Model, None)?;
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
        &history,
        &transcript,
        &mut usage,
        &mut baseline,
        &mut pen,
        first_turn,
    );
    match outcome {
        Ok(stop) => {
            settle(inner, handle, turn, stop, baseline)?;
            Ok(stop)
        }
        Err(e) => {
            keep_blocks(pen, handle);
            if stopped(handle, &e) {
                settle(inner, handle, turn, Stop::User, baseline)?;
                return Ok(Stop::User);
            }
            broken(inner, handle, turn, baseline, &e);
            Err(e)
        }
    }
}

/// Close one turn: its receipt leaves on the same queue as everything else the turn said, so a
/// front end can never see the next turn's first frame before it.
fn settle(
    inner: &Arc<Inner>,
    handle: &Handle,
    turn: i64,
    stop: Stop,
    baseline: Option<u64>,
) -> Result<()> {
    let Some(piece) = pen::turn::settle(&inner.writer, turn, stop, baseline, handle.meta())? else {
        return Ok(());
    };
    let _ = inner.out.send(Event {
        session: piece.session,
        element: Element::Structure {
            turn: piece.turn,
            ranges: piece.ranges,
            compact: piece.compact,
        },
    });
    Ok(())
}

/// One turn's laps. Every early return lands here; the caller owns the settle.
fn laps(
    inner: &Arc<Inner>,
    handle: &Arc<Handle>,
    turn: i64,
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
        // The blocking point: the last request has returned, so whatever the boxes hold is taken
        // into this turn before the next request goes out.
        take_mail(inner, handle, turn, transcript)?;
        if fence.hit() {
            return Ok(Stop::Fence);
        }

        let blocks = Arc::new(Pen::new(inner.writer.clone(), turn));
        *pen = Some(blocks.clone());
        // The session's request side, as it stands now: a switch mid-turn lands next lap.
        let config = handle.config();
        let lap = inner.ask.lap(LapInput {
            session: handle.id,
            history: history.clone(),
            blocks: blocks.clone(),
            transport: config.transport(),
            system: config.system.clone(),
            tools: config.manuals.clone(),
            tools_json: config.tools_json.clone(),
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
            // A turn is over only when the model asked for nothing and both boxes are empty. What
            // landed while this lap ran is taken here, and gets a lap of its own.
            if take_mail(inner, handle, turn, transcript)? {
                continue;
            }
            if let Some(text) = child.as_ref().and_then(|child| child.nag()) {
                pen::turn::append_user(&inner.writer, turn, None, text.to_string(), None)?;
                transcript.append_user(Stored::User {
                    prefix: None,
                    content: text.to_string(),
                    suffix: None,
                });
                continue;
            }
            return Ok(lap.stop);
        }

        let calls = lap.calls.clone();
        let mut env = handle.env();
        let results = inner.batch.run(&mut env, &calls, inner.out.clone())?;
        drop(env);
        // Calls the stop cut off never ran: their request blocks leave no trace.
        let unanswered: std::collections::BTreeSet<String> = lap
            .calls
            .iter()
            .filter(|call| !results.iter().any(|run| run.call_id == call.call_id))
            .map(|call| call.call_id.clone())
            .collect();
        blocks.drop_calls(&unanswered);
        let (children, deletes) = effects(&blocks, &results, &config)?;
        let stopped = results.iter().any(|result| result.stop);
        let reported = ResultsInput {
            session: handle.id,
            history: history.clone(),
            calls: lap.calls.clone(),
            results: results.into_iter().map(report).collect(),
        };
        inner.ask.results(reported, &*blocks)?;
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
                    provider: config.provider.clone(),
                    model: config.model.clone(),
                    system: config.system.clone(),
                    tools: spec.tools,
                    manuals: spec.manuals,
                    compact: config.compact.clone(),
                    meta: Some(meta),
                    cwd: spec.cwd,
                },
                spec.start,
                spec.answer,
            ));
            jobs.start(JobSpec {
                runner,
                schedule: Schedule::After(std::time::Duration::ZERO),
                timeout: Some(inner.task_timeout),
                output: false,
                needs_slot: true,
                kind: "agent",
                wake_on_done: true,
            })?;
        }
        if !deletes.is_empty() {
            pen::effect::apply(&inner.writer, handle.id, &deletes)?;
        }
        if stopped {
            return Ok(Stop::Finished);
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
    config: &super::super::session::config::Config,
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
                                model: config.model.clone(),
                                provider: config.provider.clone(),
                                system: config.system.clone(),
                                tools_json,
                                baseline_prompt: None,
                            },
                        },
                        Row::Session {
                            started_at: now_stamp(),
                            cwd: cwd.to_string_lossy().into_owned(),
                            model: Some((config.provider.clone(), config.model.clone())),
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
fn report(result: ToolResult) -> Reported {
    Reported {
        call_id: result.call_id,
        ok: result.ok,
        result: result.result,
        reason: result.reason,
        memo: result.memo,
        spill: result.spill,
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
    failed(inner, handle, turn, baseline, why, "the turn broke");
}

/// One round died: the watcher reads one line naming what kind of round it was, then the reason.
fn failed(
    inner: &Arc<Inner>,
    handle: &Handle,
    turn: i64,
    baseline: Option<u64>,
    why: &anyhow::Error,
    label: &str,
) {
    let _ = inner.out.send(Event {
        session: handle.id,
        element: Element::Notice(Notice {
            level: NoticeLevel::Error,
            text: format!("{label}: {why:#}"),
        }),
    });
    let _ = settle(inner, handle, turn, Stop::Broken, baseline);
}

/// Take both boxes into the running turn at a blocking point: wakes first, then the slot.
/// Every text taken is already a block row; `true` = something was taken, so the turn goes on.
fn take_mail(
    inner: &Arc<Inner>,
    handle: &Handle,
    turn: i64,
    transcript: &super::super::session::history::Transcript,
) -> Result<bool> {
    let mut taken = false;
    while let Some(text) = handle.mailbox.take_wake() {
        pen::turn::append_wake(&inner.writer, turn, text.clone())?;
        let _ = inner.out.send(Event {
            session: handle.id,
            element: Element::Stream(Stream::Wake(text.clone())),
        });
        transcript.append_user(Stored::User {
            prefix: None,
            content: text,
            suffix: None,
        });
        taken = true;
    }
    // A letter joins the turn it landed in; the writer's front end echoes it on the take signal.
    if let Some(letter) = handle.mailbox.take() {
        pen::turn::append_user(
            &inner.writer,
            turn,
            letter.prefix.clone(),
            letter.text.clone(),
            letter.suffix.clone(),
        )?;
        let _ = inner.out.send(Event {
            session: handle.id,
            element: Element::Mailbox { turn },
        });
        transcript.append_user(Stored::User {
            prefix: letter.prefix,
            content: letter.text,
            suffix: letter.suffix,
        });
        taken = true;
    }
    Ok(taken)
}
