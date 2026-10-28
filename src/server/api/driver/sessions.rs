//! One `Live` per session, built on demand: the state that outlives a round.

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::Duration;

use crate::server::agent::r#loop::budget::ToolBudget;
use crate::server::agent::r#loop::driver::Driver;
use crate::server::agent::r#loop::{Live, Settings};
use crate::server::agent::request::transport::client::Client;
use crate::server::agent::tools::McpBridge;
use crate::server::agent::tools::jobs::{JobFactory, JobService, WakeSink};
use crate::server::agent::tools::subagent::{ChildSpec, SubagentFactory};
use crate::server::agent::tools::{Env, MemoRead, TodoPhase, Workspace, roster};
use crate::server::api::call::tool::memo::MemoStore;
use crate::server::api::call::tool::todo;
use crate::server::config::read::profile::{self, ResolvedProfile};
use crate::server::config::{Config, Provider, ResolvedModel};
use crate::server::storage::{SessionChange, Submit, Writer};

const DEFAULT_MAX_TOKENS: u64 = 4096;

struct Slot {
    live: Option<Live>,
    pending: Option<String>,
    land: bool,
    instruction: String,
    profile: String,
    wakes: Arc<Mutex<Vec<String>>>,
    jobs: Option<Arc<JobService>>,
}

impl Slot {
    fn new(instruction: String, profile: String) -> Self {
        Self {
            live: None,
            pending: None,
            land: true,
            instruction,
            profile,
            wakes: Arc::new(Mutex::new(Vec::new())),
            jobs: None,
        }
    }
}

/// The session table.
#[derive(Clone)]
pub(crate) struct Sessions {
    inner: Arc<SessionsInner>,
}

pub(crate) struct SessionsInner {
    slots: Mutex<HashMap<i64, Slot>>,
    jobs: Arc<JobFactory>,
    subagent: OnceLock<Arc<SubagentFactory>>,
    mcp: Mutex<Option<Arc<dyn McpBridge>>>,
    writer: Writer,
}

impl Sessions {
    pub(crate) fn new(
        jobs: Arc<JobFactory>,
        mcp: Option<Arc<dyn McpBridge>>,
        writer: Writer,
    ) -> Self {
        Self {
            inner: Arc::new(SessionsInner {
                slots: Mutex::new(HashMap::new()),
                jobs,
                subagent: OnceLock::new(),
                mcp: Mutex::new(mcp),
                writer,
            }),
        }
    }

    pub(crate) fn install_subagent(&self, factory: Arc<SubagentFactory>) {
        let _ = self.inner.subagent.set(factory);
    }

    /// Replace the MCP bridge after a config reload; the old one shuts down.
    pub(crate) fn replace_mcp(&self, mcp: Option<Arc<dyn McpBridge>>) {
        let mut slot = self.inner.mcp.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(old) = slot.take() {
            old.shutdown();
        }
        *slot = mcp;
    }

    pub(crate) fn downgrade(&self) -> Weak<SessionsInner> {
        Arc::downgrade(&self.inner)
    }

    pub(crate) fn upgrade(inner: Weak<SessionsInner>) -> Option<Self> {
        inner.upgrade().map(|inner| Self { inner })
    }

    /// Borrow the session's `Live` for a round, building it if this is the first one.
    pub(crate) fn take(&self, cfg: &Config, conn: &Connection, session: i64) -> Result<Live> {
        let child = is_child(conn, session)?;
        let writer = self.inner.writer.clone();
        let jobs_factory = self.inner.jobs.clone();
        let subagent_factory = self.inner.subagent.get().cloned();
        let mcp = self
            .inner
            .mcp
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut slots = self.slots();
        let slot = match slots.get_mut(&session) {
            Some(slot) => slot,
            None => {
                let name = profile::active_name(cfg);
                let instruction = instruction(cfg, &name)?;
                slots.insert(session, Slot::new(instruction, name));
                slots.get_mut(&session).expect("just inserted")
            }
        };
        if !child && slot.jobs.is_none() {
            slot.jobs = Some(jobs_factory.make(session));
        }
        let jobs = if child { None } else { slot.jobs.clone() };
        let wakes = slot.wakes.clone();
        let wake_sink: Option<Arc<dyn WakeSink>> = if child {
            None
        } else {
            Some(jobs_factory.sink())
        };
        let child_spec = if child {
            let spec = subagent_factory
                .as_ref()
                .and_then(|factory| factory.child_spec(session));
            Some(spec.context("a child session has no subagent spec")?)
        } else {
            None
        };
        let is_child = child_spec.is_some();
        let chosen = slot.pending.clone().filter(|_| slot.land);
        let profile_name = chosen.clone().unwrap_or_else(|| slot.profile.clone());
        let mut live = match slot.live.take() {
            Some(live) => live,
            None => build(
                cfg,
                conn,
                session,
                Some(profile_name.as_str()),
                jobs,
                wakes,
                wake_sink,
                child_spec,
                subagent_factory,
                mcp,
            )?,
        };
        if let Some(name) = chosen {
            if !is_child {
                let resolved = profile::resolve(cfg, &name)?;
                slot.instruction = apply(&mut live, &resolved, None);
                writer.push(Submit::Session {
                    session,
                    change: SessionChange::Profile(name.clone()),
                })?;
            }
            slot.pending = None;
            slot.profile = name;
        }
        slot.land = false;
        Ok(live)
    }

    /// The round is over: put the `Live` back.
    pub(crate) fn put(&self, session: i64, live: Live) {
        let mut slots = self.slots();
        if let Some(slot) = slots.get_mut(&session) {
            if let Some(jobs) = live.jobs.clone() {
                slot.jobs = Some(jobs);
            }
            slot.live = Some(live);
        }
    }

    /// The session's working directory: the live one when it exists, else the stored row.
    pub(crate) fn cwd(&self, _cfg: &Config, conn: &Connection, session: i64) -> Result<PathBuf> {
        if let Some(cwd) = self
            .slots()
            .get(&session)
            .and_then(|slot| slot.live.as_ref())
            .map(|live| live.driver.cwd().to_path_buf())
        {
            return Ok(cwd);
        }
        let cwd: Option<String> = conn
            .query_row("SELECT cwd FROM sessions WHERE id = ?1", [session], |r| {
                r.get(0)
            })
            .optional()?
            .flatten();
        Ok(PathBuf::from(cwd.unwrap_or_default()))
    }

    /// Put the todo list back into the `Live` a rewind left behind.
    pub(crate) fn set_todo(&self, session: i64, list: Vec<TodoPhase>) {
        let mut slots = self.slots();
        if let Some(live) = slots.get_mut(&session).and_then(|slot| slot.live.as_mut()) {
            live.todo = list;
        }
    }

    /// Remember a profile chosen mid-session: it lands at the next boundary.
    pub(crate) fn choose_profile(&self, cfg: &Config, session: i64, name: &str) -> Result<()> {
        let instruction = instruction(cfg, name)?;
        let mut slots = self.slots();
        match slots.get_mut(&session) {
            Some(slot) => slot.pending = Some(name.to_string()),
            None => {
                let active = profile::active_name(cfg);
                slots.insert(session, Slot::new(instruction, active));
                slots.get_mut(&session).expect("just inserted").pending = Some(name.to_string());
            }
        }
        Ok(())
    }

    /// A compaction just happened: the chosen profile may land on the next round.
    pub(crate) fn arm_profile(&self, session: i64) {
        let mut slots = self.slots();
        if let Some(slot) = slots.get_mut(&session) {
            slot.land = true;
        }
    }

    /// The compaction instruction of the profile this session runs.
    pub(crate) fn instruction(&self, session: i64) -> Option<String> {
        self.slots()
            .get(&session)
            .map(|slot| slot.instruction.clone())
    }

    /// `/switch`: point this session's connection at another model.
    pub(crate) fn retarget(&self, cfg: &Config, session: i64, id: &str) -> Result<()> {
        let mut slots = self.slots();
        let Some(live) = slots.get_mut(&session).and_then(|slot| slot.live.as_mut()) else {
            return Ok(());
        };
        live.transport = transport(cfg, session, &model_of(cfg, id)?)?;
        Ok(())
    }

    /// Let the session's `Live` go.
    pub(crate) fn drop_live(&self, session: i64) {
        if let Some(slot) = self.slots().get_mut(&session) {
            slot.live = None;
            slot.jobs = None;
            slot.wakes.lock().unwrap_or_else(|e| e.into_inner()).clear();
        }
    }

    /// Drop every idle `Live`; the next round rebuilds from the new config.
    pub(crate) fn invalidate(&self) {
        for slot in self.slots().values_mut() {
            slot.live = None;
        }
    }

    /// The session is gone; forget everything about it.
    pub(crate) fn forget(&self, session: i64) {
        self.cancel_jobs(session);
        self.slots().remove(&session);
    }

    pub(crate) fn wake_queue(&self, session: i64) -> Option<Arc<Mutex<Vec<String>>>> {
        self.slots().get(&session).map(|slot| slot.wakes.clone())
    }

    pub(crate) fn cancel_jobs(&self, session: i64) {
        let mut slots = self.slots();
        if let Some(slot) = slots.get_mut(&session) {
            if let Some(jobs) = &slot.jobs {
                jobs.cancel_all();
            }
            if slot.live.is_none() {
                slot.jobs = None;
            }
        }
    }

    fn slots(&self) -> MutexGuard<'_, HashMap<i64, Slot>> {
        self.inner.slots.lock().unwrap_or_else(|e| e.into_inner())
    }
}

fn is_child(conn: &Connection, session: i64) -> Result<bool> {
    let child: Option<i64> = conn
        .query_row(
            "SELECT subsession_id FROM subsessions WHERE subsession_id = ?1",
            [session],
            |r| r.get(0),
        )
        .optional()?;
    Ok(child.is_some())
}

#[allow(clippy::too_many_arguments)]
fn build(
    cfg: &Config,
    conn: &Connection,
    session: i64,
    profile: Option<&str>,
    jobs: Option<Arc<JobService>>,
    wakes: Arc<Mutex<Vec<String>>>,
    wake_sink: Option<Arc<dyn WakeSink>>,
    child_spec: Option<ChildSpec>,
    subagent: Option<Arc<SubagentFactory>>,
    mcp: Option<Arc<dyn McpBridge>>,
) -> Result<Live> {
    let (cwd, model, row_profile) = row(conn, session)?;
    let name = match (child_spec.is_some(), row_profile, profile) {
        (true, Some(name), _) => name,
        (_, _, Some(name)) => name.to_string(),
        _ => profile::active_name(cfg),
    };
    let resolved = profile::resolve(cfg, &name)?;
    let addressed = match model {
        Some((provider, model)) => format!("{provider}:{model}"),
        None => {
            let d = cfg.default_model()?;
            format!("{}:{}", d.provider_name, d.entry.id)
        }
    };
    let model = model_of(cfg, &addressed)?;
    let model_pair = Some((
        model.resolved.provider_name.clone(),
        model.resolved.entry.id.clone(),
    ));
    let budget = child_spec.as_ref().map(|spec| spec.max_tool_calls);
    let mut live = Live {
        transport: transport(cfg, session, &model)?,
        driver: Driver::new(
            session,
            Workspace::new(cwd.clone()),
            ToolBudget::new(budget),
        ),
        roster: Vec::new(),
        todo: todo::list(conn, session)?,
        stop: Arc::new(AtomicBool::new(false)),
        killed: Arc::new(AtomicBool::new(false)),
        artifacts: None,
        memo: conn.path().map(|p| {
            let store: Arc<dyn MemoRead> = Arc::new(MemoStore::new(std::path::PathBuf::from(p)));
            store
        }),
        jobs,
        subagent: None,
        mcp,
        wakes,
        wake_sink,
        settings: Settings {
            system: resolved.system.clone(),
            max_output_tokens: model
                .resolved
                .entry
                .max_output_tokens
                .unwrap_or(DEFAULT_MAX_TOKENS) as u32,
            limits: cfg.app.server.agent.tools.limits(),
            browser: cfg.app.server.agent.tools.browser.clone(),
            bash_timeout: Duration::from_secs(cfg.app.server.agent.tools.bash_timeout),
            enabled: None,
            retry: Default::default(),
            profile: name.clone(),
            budget,
        },
    };
    apply(&mut live, &resolved, child_spec.as_ref());
    if let Some(factory) = subagent {
        let parent_tools: std::collections::BTreeSet<String> = live
            .roster
            .iter()
            .map(|t| t.function.name.clone())
            .collect();
        let parent_root = child_spec
            .as_ref()
            .map(|spec| spec.parent_root.clone())
            .unwrap_or_else(|| live.driver.workspace.root().to_path_buf());
        let parent_system = child_spec
            .as_ref()
            .map(|spec| spec.system.clone())
            .unwrap_or_else(|| live.settings.system.clone());
        live.subagent = Some(factory.make(
            session,
            parent_root,
            parent_tools,
            parent_system,
            name,
            model_pair,
            live.jobs.clone(),
            child_spec.is_some(),
        ));
        apply(&mut live, &resolved, child_spec.as_ref());
    }
    Ok(live)
}

fn apply(live: &mut Live, resolved: &ResolvedProfile, child: Option<&ChildSpec>) -> String {
    match child {
        Some(spec) => {
            live.settings.system = spec.system.clone();
            live.settings.enabled = Some(
                spec.tools
                    .iter()
                    .cloned()
                    .chain(std::iter::once("final".to_string()))
                    .collect(),
            );
        }
        None => {
            live.settings.system = resolved.system.clone();
            live.settings.enabled = resolved.tools.clone().map(|t| t.into_iter().collect());
        }
    }
    let enabled: Option<Vec<String>> = live
        .settings
        .enabled
        .clone()
        .map(|set| set.into_iter().collect());
    let mut env = Env::new(live.driver.cwd().to_path_buf()).with_enabled(enabled);
    env.inbox.jobs = live.jobs.clone();
    env.inbox.subagent = live.subagent.clone();
    env.mcp = live.mcp.clone();
    live.roster = roster(&env);
    resolved.compact_instruction.clone()
}

fn instruction(cfg: &Config, name: &str) -> Result<String> {
    Ok(profile::resolve(cfg, name)?.compact_instruction)
}

fn row(
    conn: &Connection,
    session: i64,
) -> Result<(PathBuf, Option<(String, String)>, Option<String>)> {
    let (cwd, provider, model, profile) = conn
        .query_row(
            "SELECT cwd, model_provider, model, profile FROM sessions WHERE id = ?1",
            [session],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .with_context(|| format!("no session {session}"))?;
    Ok((
        PathBuf::from(cwd.unwrap_or_default()),
        provider.zip(model),
        profile,
    ))
}

struct Target {
    resolved: ResolvedModel,
    provider: Provider,
}

fn model_of(cfg: &Config, addressed: &str) -> Result<Target> {
    let resolved = cfg.model_by_id(addressed)?;
    let provider = cfg
        .models
        .providers
        .get(&resolved.provider_name)
        .cloned()
        .with_context(|| format!("no provider {}", resolved.provider_name))?;
    Ok(Target { resolved, provider })
}

fn transport(cfg: &Config, session: i64, target: &Target) -> Result<Client> {
    let protocol =
        crate::server::agent::request::protocols::ProtocolId::parse(&target.provider.api)?;
    let mut client = Client::new(
        &target.provider.base_url,
        &cfg.resolve_key(&target.provider),
        &target.resolved.entry.id,
        protocol,
        target.resolved.entry.compat.as_ref(),
    )
    .with_own_worker();
    client.set_session_id(session.to_string());
    Ok(client)
}
