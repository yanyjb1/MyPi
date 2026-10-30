//! The lap cycle: ask, run the batch, take the mailboxes, settle.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::Result;

use super::ask::{CompactInput, LapInput, ResultsInput, Stop, Usage};
use super::env;
use super::fence::Fence;
use crate::server::agent::r#loop::pen::{self, Pen};
use crate::server::agent::r#loop::session::book::{Inner, Retention};
use crate::server::agent::r#loop::session::handle::Handle;
use crate::server::agent::r#loop::session::history::Transcript;
use crate::server::agent::r#loop::session::live::Live;
use crate::server::agent::r#loop::turn::batch::ToolResult;
use crate::server::agent::request::contract::{History, ToolResult as Reported};

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
    if let Mode::Compact { retention, focus } = mode {
        let reply = match inner.ask.compact(CompactInput {
            session: handle.id,
            history: Arc::new(Transcript::open(&inner.writer.path())?),
            transport: live.transport(),
            system: live.system.clone(),
            tools: live.tools.clone(),
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
            tools: live.tools.clone(),
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
            if handle.mailbox.is_empty() {
                stop = lap.stop;
                break;
            }
            continue;
        }

        let input = env::build(
            handle.id,
            turn,
            live.cwd.clone(),
            lap.calls.clone(),
            handle.stop.clone(),
            inner.out.clone(),
            live.artifacts.clone(),
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
            artifacts: live.artifacts.dir().to_path_buf(),
            calls: lap.calls.clone(),
            results: results.iter().map(report).collect(),
        };
        if let Err(e) = inner.ask.results(reported, &*blocks) {
            broken(inner, handle, turn, usage, baseline);
            return Err(e);
        }
        let effects: Vec<_> = results
            .iter()
            .flat_map(|result| result.effects.iter().cloned())
            .collect();
        if !effects.is_empty() {
            for mut born in pen::effect::write(&inner.writer, &effects)? {
                born.birth.meta = handle.meta();
                born.birth.compact = live.compact.clone();
                crate::server::agent::r#loop::session::book::book_child(
                    inner, born.birth, born.start,
                )?;
            }
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
