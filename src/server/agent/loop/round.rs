use anyhow::Result;
use rusqlite::Connection;

use super::Live;
use super::Turn;
use super::outlet::{Outlet, RetryToOutlet};
use crate::server::agent::request::content::RequestContent;
use crate::server::agent::request::message::{StopReason, Usage};
use crate::server::agent::request::{self};
use crate::server::agent::tools::Env;
use crate::server::storage::{Submit, Writer};

/// How one lap ends.
pub enum Verdict {
    Done(StopReason),
    Tools(usize),
}

/// One lap's result.
pub struct Lap {
    pub verdict: Verdict,
    pub usage: Usage,
}

/// Run one request and, when asked, its tools.
pub fn once(
    w: &Writer,
    r: &Connection,
    live: &mut Live,
    env: &mut Env,
    turn: &Turn,
    out: &mut dyn Outlet,
    content: &RequestContent,
    first: bool,
) -> Result<Lap> {
    let answer = {
        let mut retry = RetryToOutlet::new(out);
        request::start(
            w,
            r,
            &live.transport,
            content,
            &mut retry,
            live.stop.as_ref(),
        )?
    };

    if first {
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

    let ran = live
        .driver
        .run_calls(w, env, &answer.message, out, Some(turn.turn_id))?;
    let stopped = ran.iter().any(|r| r.stop);
    request::hand_results(w, turn.session, &answer.message, &ran)?;
    if let Some(subagent) = live.subagent.as_ref() {
        for run in &ran {
            subagent.activate_effects(&run.effects);
        }
    }
    Ok(Lap {
        verdict: if stopped {
            Verdict::Done(StopReason::Stop)
        } else {
            Verdict::Tools(ran.len())
        },
        usage: answer.usage,
    })
}
