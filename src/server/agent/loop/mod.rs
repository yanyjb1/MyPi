//! The loop. One turn is one loop: send a request, run whatever tools the model asks for, hand out
//! the stop signal. No context of its own, writes first, tools one at a time.

mod env;
mod round;

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::Result;
use rusqlite::Connection;

use crate::server::agent::request::content::RequestContent;
use crate::server::agent::request::message::{StopReason, ToolDef, Usage};
use crate::server::agent::request::transport::client::Client;
use crate::server::agent::tools::{Artifacts, TodoPhase, Workspace};
use crate::server::config::{BrowserConfig, OutputLimits};
use crate::server::storage::{RoundUsage, Submit, Writer};

/// What the main loop holds: everything that outlives one turn, keyed by session.
pub struct Live {
    /// The provider connection. Swapped when the model changes.
    pub transport: Client,
    /// The working directory. In-memory truth: the model's `cd` changes it, nothing persists it.
    pub workspace: Workspace,
    /// The tool list. Decided at the session's first turn and after each compaction; constant within
    /// one turn.
    pub roster: Vec<ToolDef>,
    /// The list. Held by the main loop; empty here while the session has used todo means replay it.
    pub todo: Vec<TodoPhase>,
    /// The session's stop flag, set when the user presses Esc.
    pub stop: Arc<AtomicBool>,
    /// Artifact fetch. Built with the first Env and lives with the session: when the session is
    /// dropped (this `Live` goes), its temp files go too.
    pub artifacts: Option<Arc<dyn Artifacts>>,
    /// The config values in force.
    pub settings: Settings,
}

/// The config values one turn needs. The caller reads them out of config.
pub struct Settings {
    /// The system prompt in force.
    pub system: String,
    /// The output cap for this turn.
    pub max_tokens: u32,
    /// The output budget (`read` and `bash`).
    pub limits: OutputLimits,
    /// The bundle the three web tools need.
    pub browser: BrowserConfig,
    /// A command's wall-clock limit.
    pub bash_timeout: Duration,
    /// The profile gate: `None` = every tool.
    pub enabled: Option<BTreeSet<String>>,
    /// Fence: how many tool rounds one turn may run; `None` = off. **Keep it**: subagents will cap
    /// tool calls with it.
    pub fence: Option<usize>,
}

/// This turn.
pub struct Turn {
    /// The session id.
    pub session: i64,
    /// The turn id. api opened it when it filed the opening batch (the user's message).
    pub turn_id: i64,
}

/// The four endings the loop sees. Filing them is storage's business.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// The model stopped itself: no more tools.
    Model,
    /// The user interrupted (the stop flag was set).
    User,
    /// The network died, or something failed.
    Broken,
    /// The tool-round fence was hit.
    Fenced,
}

/// What the turn hands out once it is over.
pub struct Ended {
    /// Why it stopped.
    pub stop: Stop,
    /// Every round of this turn, added up.
    pub usage: RoundUsage,
}

/// The loop's outward line: deltas, plus the three moments of one tool call.
pub trait Outlet {
    /// Text delta. `false` = stop.
    fn content(&mut self, text: &str) -> bool;
    /// Reasoning delta. `false` = stop.
    fn reasoning(&mut self, text: &str) -> bool;
    /// The tool said something while running (a slice of the result).
    fn tool_output(&mut self, call_id: &str, text: &str);
    /// The three moments of one tool call.
    fn tool_event(&mut self, event: &ToolEvent);
}

/// The three moments of one tool call are outward-facing. The loop forwards them as is.
#[derive(Debug, Clone)]
pub enum ToolEvent {
    /// A call began. `args` is the JSON string exactly as the model sent it.
    Start {
        call_id: String,
        name: String,
        args: String,
        intent: String,
        text: String,
        first: bool,
    },
    /// It said something mid-run.
    Progress { call_id: String, text: String },
    /// It is over. `result` goes to the model, `memo` to front ends; `duration_ms` is measured here.
    Finish {
        call_id: String,
        name: String,
        ok: bool,
        result: String,
        memo: Option<serde_json::Value>,
        duration_ms: u64,
    },
}

/// Run one turn: from the start signal to the stop signal. `w` writes, `r` is the caller's read
/// connection (the loop opens none). The turn row and the user's message are already filed.
pub fn run(
    w: &Writer,
    r: &Connection,
    live: &mut Live,
    turn: Turn,
    out: &mut dyn Outlet,
) -> Result<Ended> {
    // Workspace and list move into the Env: `cd` changes one, todo writes back the other. At the end
    // of the turn they go back into `Live`, ready for the next one.
    let mut env = env::build(r, live, turn.session);

    let mut usage = RoundUsage::default();
    let mut tool_rounds = 0usize;
    let mut rounds = 0usize;

    let stop = loop {
        if live.stop.load(Ordering::Relaxed) {
            break Stop::User;
        }

        // Fence hit: ask once more, this time **without the tool manual** — the model can only finish
        // what it was saying. A tool call it still writes is repaired at assembly.
        let fenced = live
            .settings
            .fence
            .is_some_and(|limit| tool_rounds >= limit);

        let lap = match round::once(w, r, live, &mut env, &turn, out, rounds, fenced) {
            Ok(lap) => lap,
            // Close out even on an error: blocks already filed stay, and the turn ends here.
            Err(_) => break Stop::Broken,
        };
        rounds += 1;
        add(&mut usage, &lap.usage);

        if fenced {
            break Stop::Fenced;
        }
        match lap.verdict {
            round::Verdict::Done(why) => break done(why),
            round::Verdict::Tools(ran) => tool_rounds += ran,
        }
    };

    env::restore(live, env);
    // Closing out: why it stopped, how much it used. Filing that is storage's business.
    w.push(Submit::RoundEnd {
        session: turn.session,
        stop_reason: Some(reason(stop).to_string()),
        usage: Some(usage),
    })?;
    Ok(Ended { stop, usage })
}

/// One request's input: just the session id; the user's message is read back from the list.
pub(crate) fn content(live: &Live, session: i64, fenced: bool) -> RequestContent {
    RequestContent {
        session,
        system: live.settings.system.clone(),
        tools: if fenced {
            Vec::new()
        } else {
            live.roster.clone()
        },
        max_tokens: live.settings.max_tokens,
    }
}

/// The reply's own stop reason → the ending the loop sees.
fn done(why: StopReason) -> Stop {
    match why {
        StopReason::Interrupted => Stop::User,
        _ => Stop::Model,
    }
}

/// The ending's spelling (`turns.stop_reason`).
fn reason(stop: Stop) -> &'static str {
    match stop {
        Stop::Model => "stop",
        Stop::User => "interrupted",
        Stop::Broken => "broken",
        Stop::Fenced => "fenced",
    }
}

/// All rounds' usage, added up.
fn add(total: &mut RoundUsage, one: &Usage) {
    total.prompt_tokens += one.prompt_tokens;
    total.completion_tokens += one.completion_tokens;
    total.cached_tokens += one.cached_tokens.unwrap_or(0);
    total.reasoning_tokens += one.reasoning_tokens.unwrap_or(0);
    total.cache_write_tokens += one.cache_write_tokens.unwrap_or(0);
}
