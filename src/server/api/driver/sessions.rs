//! The session table: one `Live` per session, built on demand. A `Live` is the state that outlives a
//! round — the provider connection, the working directory, the roster, the todo list, the settings.

use anyhow::{Context, Result};
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use crate::server::agent::r#loop::{Live, Settings};
use crate::server::agent::request::transport::client::Client;
use crate::server::agent::tools::{Env, TodoPhase, Workspace, roster};
use crate::server::api::call::todo;
use crate::server::config::read::profile::{self, ResolvedProfile};
use crate::server::config::{Config, Provider, ResolvedModel};

/// The output ceiling for a model that declares none (`maxOutputTokens` absent).
const DEFAULT_MAX_TOKENS: u64 = 4096;

/// One session's slot: its `Live` (when no round is holding it), the profile
/// chosen but not yet in effect, and whether that profile may land.
struct Slot {
    live: Option<Live>,
    pending: Option<String>,
    /// May a pending profile land on the next round? True for a session's first
    /// round, and set again by a compaction.
    land: bool,
    /// The compaction instruction of the profile in effect.
    instruction: String,
}

impl Slot {
    fn new(instruction: String) -> Self {
        Self {
            live: None,
            pending: None,
            land: true,
            instruction,
        }
    }
}

/// The session table.
#[derive(Clone, Default)]
pub(crate) struct Sessions {
    slots: Arc<Mutex<HashMap<i64, Slot>>>,
}

impl Sessions {
    /// Borrow the session's `Live` for a round, building it if this is the first one.
    pub(crate) fn take(&self, cfg: &Config, conn: &Connection, session: i64) -> Result<Live> {
        let mut slots = self.slots();
        let slot = match slots.get_mut(&session) {
            Some(slot) => slot,
            None => {
                let instruction = instruction(cfg, &profile::active_name(cfg))?;
                slots.insert(session, Slot::new(instruction));
                slots.get_mut(&session).expect("just inserted")
            }
        };
        let chosen = slot.pending.clone().filter(|_| slot.land);
        let mut live = match slot.live.take() {
            Some(live) => live,
            None => build(cfg, conn, session, chosen.as_deref())?,
        };
        // A profile chosen while the session was running takes effect here.
        if let Some(name) = chosen {
            let resolved = profile::resolve(cfg, &name)?;
            slot.instruction = apply(&mut live, &resolved);
            slot.pending = None;
        }
        slot.land = false;
        Ok(live)
    }

    /// The round is over: put the `Live` back.
    pub(crate) fn put(&self, session: i64, live: Live) {
        let mut slots = self.slots();
        if let Some(slot) = slots.get_mut(&session) {
            slot.live = Some(live);
        }
    }

    /// The session's working directory in memory, building the `Live` if this
    /// session has not been touched yet.
    pub(crate) fn cwd(&self, cfg: &Config, conn: &Connection, session: i64) -> Result<PathBuf> {
        let live = self.take(cfg, conn, session)?;
        let cwd = live.workspace.cwd().to_path_buf();
        self.put(session, live);
        Ok(cwd)
    }

    /// The session's todo list as it stands, put back into the `Live` a rewind
    /// left behind (the cut may have taken the rounds it was built from).
    pub(crate) fn set_todo(&self, session: i64, list: Vec<TodoPhase>) {
        let mut slots = self.slots();
        if let Some(live) = slots.get_mut(&session).and_then(|slot| slot.live.as_mut()) {
            live.todo = list;
        }
    }

    /// Remember a profile chosen mid-session: it lands at the next moment.
    pub(crate) fn choose_profile(&self, cfg: &Config, session: i64, name: &str) -> Result<()> {
        let instruction = instruction(cfg, name)?;
        let mut slots = self.slots();
        match slots.get_mut(&session) {
            Some(slot) => slot.pending = Some(name.to_string()),
            None => {
                slots.insert(session, Slot::new(instruction));
                slots
                    .get_mut(&session)
                    .expect("just inserted")
                    .pending = Some(name.to_string());
            }
        }
        Ok(())
    }

    /// A compaction just happened: the chosen profile may land on the next
    /// round.
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

    /// `/switch`: point this session's connection at another model. The row is
    /// written by the post; this is the running session catching up.
    pub(crate) fn retarget(&self, cfg: &Config, session: i64, id: &str) -> Result<()> {
        let mut slots = self.slots();
        let Some(live) = slots.get_mut(&session).and_then(|slot| slot.live.as_mut()) else {
            return Ok(());
        };
        live.transport = transport(cfg, session, &model_of(cfg, id)?)?;
        Ok(())
    }

    /// Let the session's `Live` go. What the slot remembers besides it — a profile chosen
    /// mid-session, the compaction instruction — stays.
    pub(crate) fn drop_live(&self, session: i64) {
        if let Some(slot) = self.slots().get_mut(&session) {
            slot.live = None;
        }
    }

    /// The session is gone; forget everything about it.
    pub(crate) fn forget(&self, session: i64) {
        self.slots().remove(&session);
    }

    fn slots(&self) -> MutexGuard<'_, HashMap<i64, Slot>> {
        self.slots.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Build a session's `Live` from its row, the config and its transcript.
fn build(cfg: &Config, conn: &Connection, session: i64, profile: Option<&str>) -> Result<Live> {
    let (cwd, model) = row(conn, session)?;
    let name = match profile {
        Some(name) => name.to_string(),
        None => profile::active_name(cfg),
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
        workspace: Workspace::new(cwd),
        roster: Vec::new(),
        // The list is session state, and the transcript is its only authority:
        // a session opened again gets it back by replaying its own calls.
        todo: todo::list(conn, session)?,
        stop: Arc::new(AtomicBool::new(false)),
        // The artifact handle is built the first time a round needs the tool
        // environment (`loop::env::build`): it caches nothing until then.
        artifacts: None,
        settings: Settings {
            system: resolved.system.clone(),
            max_tokens: model
                .resolved
                .entry
                .max_output_tokens
                .unwrap_or(DEFAULT_MAX_TOKENS) as u32,
            limits: cfg.app.server.agent.tools.limits(),
            browser: cfg.app.server.agent.tools.browser.clone(),
            bash_timeout: Duration::from_secs(cfg.app.server.agent.tools.bash_timeout),
            enabled: None,
            fence: cfg.app.server.agent.r#loop.max_tool_calls.limit(),
        },
    };
    apply(&mut live, &resolved);
    Ok(live)
}

/// Put a resolved profile into a `Live`: the system prompt, the tool roster and
/// the profile's gate over it. Returns the profile's compaction instruction.
fn apply(live: &mut Live, resolved: &ResolvedProfile) -> String {
    live.settings.system = resolved.system.clone();
    live.settings.enabled = resolved.tools.clone().map(|t| t.into_iter().collect());
    let env = Env::new(live.workspace.cwd().to_path_buf()).with_enabled(resolved.tools.clone());
    live.roster = roster(&env);
    resolved.compact_instruction.clone()
}

/// The compaction instruction of one profile, resolved.
fn instruction(cfg: &Config, name: &str) -> Result<String> {
    Ok(profile::resolve(cfg, name)?.compact_instruction)
}

/// The session's row: where it works, and which model it is on.
fn row(conn: &Connection, session: i64) -> Result<(PathBuf, Option<(String, String)>)> {
    let (cwd, provider, model) = conn
        .query_row(
            "SELECT cwd, model_provider, model FROM sessions WHERE id = ?1",
            [session],
            |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .with_context(|| format!("no session {session}"))?;
    Ok((
        PathBuf::from(cwd.unwrap_or_default()),
        provider.zip(model),
    ))
}

/// A model plus the provider it lives under.
struct Target {
    resolved: ResolvedModel,
    provider: Provider,
}

/// Resolve `<provider>:<id>` against the config.
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

/// The provider connection for this session: one worker per session.
fn transport(cfg: &Config, session: i64, target: &Target) -> Result<Client> {
    let mut client = Client::new(
        &target.provider.base_url,
        &cfg.resolve_key(&target.provider),
        &target.resolved.entry.id,
    )
    .with_compat(target.resolved.entry.compat.as_ref())
    .with_own_worker();
    client.set_session_id(session.to_string());
    Ok(client)
}
