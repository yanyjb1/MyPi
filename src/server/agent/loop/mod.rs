mod artifacts;
mod env;
pub mod register;
mod round;

pub mod budget;
pub mod driver;
pub mod outlet;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use rusqlite::Connection;

use crate::server::agent::request::content::RequestContent;
use crate::server::agent::request::message::{StopReason, ToolDef, Usage};
use crate::server::agent::request::retry::RetryPolicy;
use crate::server::agent::request::transport::client::Client;
use crate::server::agent::tools::Artifacts;
use crate::server::agent::tools::jobs::{JobService, WakeSink};
use crate::server::config::{BrowserConfig, OutputLimits};
use crate::server::storage::{BlockKind, NewBlock, RoundUsage, Submit, Writer};

use driver::Driver;
use outlet::Outlet;

/// Session state that outlives one turn.
pub struct Live {
    pub transport: Client,
    pub driver: Driver,
    pub roster: Vec<ToolDef>,
    pub stop: Arc<AtomicBool>,
    pub killed: Arc<AtomicBool>,
    pub artifacts: Option<Arc<dyn Artifacts>>,
    pub jobs: Option<Arc<JobService>>,
    pub mcp: Option<Arc<dyn crate::server::agent::tools::McpBridge>>,
    pub wakes: Arc<Mutex<Vec<String>>>,
    pub wake_sink: Option<Arc<dyn WakeSink>>,
    pub settings: Settings,
}

/// Config values one turn needs.
pub struct Settings {
    pub system: String,
    pub max_output_tokens: u32,
    pub limits: OutputLimits,
    pub browser: BrowserConfig,
    pub bash_timeout: Duration,
    pub enabled: Option<BTreeSet<String>>,
    pub retry: RetryPolicy,
    pub profile: String,
    pub budget: Option<u32>,
}

/// One turn.
pub struct Turn {
    pub session: i64,
    pub turn_id: i64,
}

/// Why a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    Model,
    User,
    Broken,
    Killed,
}

/// What a finished turn returns.
pub struct Ended {
    pub stop: Stop,
    pub usage: RoundUsage,
}

/// Run one turn.
pub fn run(
    w: &Writer,
    r: &Connection,
    live: &mut Live,
    turn: Turn,
    out: &mut dyn Outlet,
) -> Result<Ended> {
    let mut env = env::build(r, live, turn.session);
    let mut usage = RoundUsage::default();
    let mut first = true;

    let stop = loop {
        if live.killed.load(Ordering::Relaxed) {
            break Stop::Killed;
        }
        if live.stop.load(Ordering::Relaxed) {
            break Stop::User;
        }

        if drain_wakes(w, &turn, live).is_err() {
            break Stop::Broken;
        }
        let content = content(live, turn.session);
        let lap = match round::once(w, r, live, &mut env, &turn, out, &content, first) {
            Ok(lap) => lap,
            Err(_) => break Stop::Broken,
        };
        first = false;
        add(&mut usage, &lap.usage);
        if live.driver.budget.wake_once() {
            out.budget_exhausted();
        }
        match lap.verdict {
            round::Verdict::Done(why) => break done(why),
            round::Verdict::Tools(_) => {}
        }
    };

    w.push(Submit::RoundEnd {
        session: turn.session,
        stop_reason: Some(reason(stop).to_string()),
        usage: Some(usage),
    })?;
    Ok(Ended { stop, usage })
}

/// Put queued wakes into the round before the next request.
fn drain_wakes(w: &Writer, turn: &Turn, live: &mut Live) -> Result<()> {
    let Some(text) = live.take_wakes() else {
        return Ok(());
    };
    let block = NewBlock {
        kind: BlockKind::Wake,
        content: Some(text),
        prefix: Some("[system wake: ".to_string()),
        suffix: Some("]".to_string()),
        ts: Some(crate::server::platform::time::now_stamp()),
        ..NewBlock::default()
    };
    w.push(Submit::Blocks {
        session: turn.session,
        blocks: vec![block],
        done: true,
        effects: Vec::new(),
    })?;
    Ok(())
}

impl Live {
    pub(crate) fn take_wakes(&self) -> Option<String> {
        let mut q = self.wakes.lock().unwrap_or_else(|e| e.into_inner());
        if q.is_empty() {
            None
        } else {
            Some(q.drain(..).collect::<Vec<_>>().join("\n"))
        }
    }
}

/// Build one request's input.
pub(crate) fn content(live: &Live, session: i64) -> RequestContent {
    let tools = if live.driver.budget.final_only() {
        live.roster
            .iter()
            .filter(|t| t.function.name == "final")
            .cloned()
            .collect()
    } else {
        live.roster.clone()
    };
    RequestContent {
        session,
        system: live.settings.system.clone(),
        tools,
        max_output_tokens: live.settings.max_output_tokens,
        profile: live.settings.profile.clone(),
    }
}

/// The reply's stop reason as a loop ending.
fn done(why: StopReason) -> Stop {
    match why {
        StopReason::Interrupted => Stop::User,
        StopReason::Error => Stop::Broken,
        _ => Stop::Model,
    }
}

/// The stored spelling of an ending.
fn reason(stop: Stop) -> &'static str {
    match stop {
        Stop::Model => "stop",
        Stop::User => "interrupted",
        Stop::Broken => "broken",
        Stop::Killed => "killed",
    }
}

/// Add one round's usage into the turn total.
fn add(total: &mut RoundUsage, one: &Usage) {
    total.prompt_tokens += one.prompt_tokens;
    total.completion_tokens += one.completion_tokens;
    total.cached_tokens += one.cached_tokens.unwrap_or(0);
    total.cache_write_tokens += one.cache_write_tokens.unwrap_or(0);
}
