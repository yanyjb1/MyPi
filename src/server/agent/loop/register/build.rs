// Live assembly: the session runtime built on demand from its stored row.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, Result};
use rusqlite::Connection;

use crate::server::agent::r#loop::budget::ToolBudget;
use crate::server::agent::r#loop::driver::Driver;
use crate::server::agent::r#loop::{Live, Settings};
use crate::server::agent::request::transport::client::Client;
use crate::server::agent::tools::jobs::JobService;
use crate::server::agent::tools::{Env, Workspace, roster};
use crate::server::config::read::profile::{self, ResolvedProfile};
use crate::server::config::{Config, Provider, ResolvedModel};
use crate::server::storage::{MemoScope, Reader, SessionChange, Submit};

use super::{Handle, Inner, Slot};

const DEFAULT_MAX_TOKENS: u64 = 4096;

pub(crate) fn instruction(cfg: &Config, name: &str) -> Result<String> {
    Ok(profile::resolve(cfg, name)?.compact_instruction)
}

/// Build or reuse the session's Live, landing a pending profile when due.
pub(crate) fn build(handle: &Handle, inner: &Inner, conn: &Connection) -> Result<Live> {
    let cfg = &inner.cfg;
    let mut slot = handle.slot();
    let slot = slot.get_or_insert_with(|| {
        let name = profile::active_name(cfg);
        let text = instruction(cfg, &name).unwrap_or_default();
        Slot::new(text, name)
    });
    if slot.jobs.is_none() {
        slot.jobs = Some(inner.jobs.make(handle.id));
    }
    let jobs = slot.jobs.clone();
    let wakes = slot.wakes.clone();
    let chosen = slot.pending.clone().filter(|_| slot.land);
    let profile_name = chosen.clone().unwrap_or_else(|| slot.profile.clone());
    let mut live = match slot.live.take() {
        Some(live) => live,
        None => assemble(cfg, conn, handle.id, &profile_name, jobs, wakes)?,
    };
    if let Some(name) = chosen {
        let resolved = profile::resolve(cfg, &name)?;
        slot.instruction = apply(&mut live, &resolved);
        inner.writer.push(Submit::Session {
            session: handle.id,
            change: SessionChange::Profile(name.clone()),
        })?;
        slot.pending = None;
        slot.profile = name;
    }
    slot.land = false;
    Ok(live)
}

#[allow(clippy::too_many_arguments)]
fn assemble(
    cfg: &Config,
    conn: &Connection,
    session: i64,
    profile_name: &str,
    jobs: Option<Arc<JobService>>,
    wakes: Arc<Mutex<Vec<String>>>,
) -> Result<Live> {
    let (cwd, model, row_profile) = row(conn, session)?;
    let name = match (row_profile, Some(profile_name)) {
        (_, Some(name)) => name.to_string(),
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
    let mut live = Live {
        transport: transport(cfg, session, &model)?,
        driver: Driver::new(session, Workspace::new(cwd.clone()), ToolBudget::new(None)),
        roster: Vec::new(),
        stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        killed: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        artifacts: None,
        jobs,
        mcp: inner_mcp(conn)?,
        wakes,
        wake_sink: None,
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
            budget: None,
        },
    };
    apply(&mut live, &resolved);
    Ok(live)
}

fn inner_mcp(
    _conn: &Connection,
) -> Result<Option<Arc<dyn crate::server::agent::tools::McpBridge>>> {
    Ok(None)
}

pub(crate) fn apply(live: &mut Live, resolved: &ResolvedProfile) -> String {
    live.settings.system = resolved.system.clone();
    live.settings.enabled = resolved.tools.clone().map(|t| t.into_iter().collect());
    let enabled: Option<Vec<String>> = live
        .settings
        .enabled
        .clone()
        .map(|set| set.into_iter().collect());
    let mut env = Env::new(live.driver.cwd().to_path_buf()).with_enabled(enabled);
    env.inbox.jobs = live.jobs.clone();
    env.mcp = live.mcp.clone();
    live.roster = roster(&env);
    resolved.compact_instruction.clone()
}

fn row(
    conn: &Connection,
    session: i64,
) -> Result<(std::path::PathBuf, Option<(String, String)>, Option<String>)> {
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
        std::path::PathBuf::from(cwd.unwrap_or_default()),
        provider.zip(model),
        profile,
    ))
}

pub(crate) struct Target {
    pub(crate) resolved: ResolvedModel,
    pub(crate) provider: Provider,
}

pub(crate) fn model_of(cfg: &Config, addressed: &str) -> Result<Target> {
    let resolved = cfg.model_by_id(addressed)?;
    let provider = cfg
        .models
        .providers
        .get(&resolved.provider_name)
        .cloned()
        .with_context(|| format!("no provider {}", resolved.provider_name))?;
    Ok(Target { resolved, provider })
}

pub(crate) fn transport(cfg: &Config, session: i64, target: &Target) -> Result<Client> {
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
