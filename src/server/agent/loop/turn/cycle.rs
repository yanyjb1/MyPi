//! The lap cycle: ask, run the batch, take the mailboxes, settle.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use anyhow::Result;

use super::ask::{CompactInput, LapInput, Stop, Usage};
use super::env;
use super::fence::Fence;
use crate::server::agent::r#loop::pen::{self, Pen};
use crate::server::agent::r#loop::session::book::{Inner, Retention};
use crate::server::agent::r#loop::session::handle::Handle;
use crate::server::agent::r#loop::session::live::Live;
use crate::server::storage::Reader;

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
        let blocks = Arc::new(Pen::new(inner.writer.clone(), turn));
        let reader = Reader::open(&inner.writer.path())?;
        let reply = match inner.ask.compact(CompactInput {
            session: handle.id,
            turn,
            reader,
            transport: live.transport(),
            system: live.system.clone(),
            tools: live.tools.clone(),
            retention,
            focus,
            stop: handle.stop.clone(),
            out: inner.out.clone(),
            blocks: blocks.clone(),
            artifacts: live.artifacts.clone(),
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
        let reader = Reader::open(&inner.writer.path())?;
        let lap = match inner.ask.lap(LapInput {
            session: handle.id,
            turn,
            reader,
            transport: live.transport(),
            system: live.system.clone(),
            tools: live.tools.clone(),
            max_output_tokens: live.settings.max_output_tokens,
            stop: handle.stop.clone(),
            out: inner.out.clone(),
            blocks: blocks.clone(),
            artifacts: live.artifacts.clone(),
            first,
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
        if let Err(e) = inner
            .ask
            .hand_results(handle.id, turn, &lap.calls, &results, &*blocks)
        {
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
