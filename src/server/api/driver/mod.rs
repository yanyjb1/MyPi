//! The driver: the state every door is reached through.

pub(crate) mod running;
pub(crate) mod sessions;

mod child;
mod compact;
mod relay;
mod round;
mod subscriptions;
mod wake;

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex, OnceLock};

use crate::ipc::contract::Viewer;
use crate::server::agent::r#loop::Live;
use crate::server::agent::request::assembly::compact as cut;
use crate::server::agent::request::assembly::history;
use crate::server::agent::request::message::{Context as Wire, Message};
use crate::server::agent::tools::jobs::{JobFactory, WakeSink};
use crate::server::agent::tools::subagent::{Spawn, SubagentFactory};
use crate::server::agent::tools::{McpBridge, SyncMcpBridge};
use crate::server::api::call;
use crate::server::api::post;
use crate::server::api::subscribe::compact::COMPACT_MESSAGE;
use crate::server::api::subscribe::event::{Event, StartKind};
use crate::server::api::subscribe::{Opened, Retention};
use crate::server::config::Config;
use crate::server::storage::{Cut, Reader, SessionRef, Submit, Target, Writer};

use relay::Relay;
use running::Running;
use sessions::Sessions;
use wake::ChannelSink;

fn mcp_bridge(cfg: &Config) -> Option<Arc<dyn McpBridge>> {
    if cfg.mcp.servers.is_empty() {
        return None;
    }
    let bridge: Arc<dyn McpBridge> = Arc::new(SyncMcpBridge::new(cfg.mcp.clone()));
    Some(bridge)
}

static STATE: OnceLock<Mutex<Driver>> = OnceLock::new();
static BUILDING: Mutex<()> = Mutex::new(());

pub(crate) fn state(cfg: &Config) -> Result<&'static Mutex<Driver>> {
    if let Some(state) = STATE.get() {
        return Ok(state);
    }
    let _building = BUILDING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(state) = STATE.get() {
        return Ok(state);
    }
    let db = crate::server::storage::default_path();
    let driver = Driver::open(cfg.clone(), &db)?;
    Ok(STATE.get_or_init(|| Mutex::new(driver)))
}

pub(crate) fn state_ready() -> Option<&'static Mutex<Driver>> {
    STATE.get()
}

/// The api's state, and everything it holds.
pub(crate) struct Driver {
    pub(crate) cfg: Config,
    pub(crate) db: PathBuf,
    pub(crate) writer: Writer,
    pub(crate) sessions: Sessions,
    pub(crate) running: Running,
    pub(crate) relay: Relay,
}

impl Driver {
    fn open(cfg: Config, db: &Path) -> Result<Driver> {
        let writer = Writer::open(db, cfg.app.server.storage.batch_tools)?;
        let relay = Relay::start(&writer)?;
        let (wake_tx, wake_rx) = std::sync::mpsc::channel();
        let sink: Arc<dyn WakeSink> = Arc::new(ChannelSink(wake_tx));
        let jobs = Arc::new(JobFactory::new(cfg.app.server.agent.tools.kairos, sink));
        let mcp = mcp_bridge(&cfg);
        let sessions = Sessions::new(jobs, mcp, writer.clone());
        let running = Running::default();
        let child = Arc::new(child::ChildSpawn::new(
            sessions.downgrade(),
            writer.clone(),
            db.to_path_buf(),
            cfg.clone(),
            running.clone(),
        ));
        let spawn: Arc<dyn Spawn> = child;
        let subagent = Arc::new(SubagentFactory::new(
            cfg.app.server.agent.subagent.max_tool_calls,
            writer.clone(),
            spawn,
        ));
        sessions.install_subagent(subagent);
        wake::spawn_worker(wake_rx);
        Ok(Driver {
            cfg,
            db: db.to_path_buf(),
            writer,
            sessions,
            running,
            relay,
        })
    }

    /// Rebuild the MCP bridge after a config reload.
    pub(crate) fn reload_mcp(&self) {
        self.sessions.replace_mcp(mcp_bridge(&self.cfg));
    }

    /// Register a viewer's one stream.
    pub(crate) fn connect(&self, viewer: &Viewer) -> Receiver<Event> {
        self.relay.connect(viewer)
    }

    /// Watch one session's whole stream.
    pub(crate) fn watch(&self, viewer: &Viewer, session: i64) -> Result<()> {
        self.relay.watch(viewer, session, None)
    }

    /// Let a session's `Live` go when nobody watches it and nothing runs in it.
    pub(crate) fn reap(&self, session: i64) {
        if !self.relay.watched(session) {
            self.sessions.cancel_jobs(session);
            if self.running.flag(session).is_none() {
                self.sessions.drop_live(session);
            }
        }
    }

    /// Land the user's message and put the round on a thread.
    pub(crate) fn submit(
        &self,
        viewer: &Viewer,
        session: i64,
        prefix: Option<&str>,
        text: &str,
        suffix: Option<&str>,
    ) -> Result<Opened> {
        let conn = Reader::open(&self.db)?;
        let child: Option<i64> = conn
            .query_row(
                "SELECT subsession_id FROM subsessions WHERE subsession_id = ?1",
                [session],
                |r| r.get(0),
            )
            .optional()?;
        Reader::close(conn);
        if child.is_some() {
            anyhow::bail!("a child session does not accept submits");
        }
        let Some(stop) = self.running.claim(session) else {
            return Ok(Opened::Busy);
        };
        let turn = match self.open_round(session, stop, prefix, text, suffix) {
            Ok(turn) => turn,
            Err(e) => {
                self.running.release(session);
                return Err(e);
            }
        };
        if let Err(e) = self.relay.watch(viewer, session, Some(turn)) {
            crate::server::platform::log::warn(
                "api",
                format!("session {session}: the submitter's stream did not attach: {e:#}"),
            );
        }
        self.relay.started(session, turn, StartKind::Submit);
        Ok(Opened::Started { turn })
    }

    /// Land the fixed compact message, then run the summarization.
    pub(crate) fn compact(
        &self,
        viewer: &Viewer,
        session: i64,
        retention: Retention,
        focus: Option<String>,
    ) -> Result<Opened> {
        let conn = Reader::open(&self.db)?;
        let child: Option<i64> = conn
            .query_row(
                "SELECT subsession_id FROM subsessions WHERE subsession_id = ?1",
                [session],
                |r| r.get(0),
            )
            .optional()?;
        Reader::close(conn);
        if child.is_some() {
            anyhow::bail!("a child session does not accept compaction");
        }
        let Some(stop) = self.running.claim(session) else {
            return Ok(Opened::Busy);
        };
        let mut job = match self.plan(session, retention, focus, stop) {
            Ok(Some(job)) => job,
            Ok(None) => {
                self.running.release(session);
                return Ok(Opened::Busy);
            }
            Err(e) => {
                self.running.release(session);
                return Err(e);
            }
        };
        let turn = match self.land_user(session, None, COMPACT_MESSAGE, None) {
            Ok(turn) => turn,
            Err(e) => {
                self.sessions.put(session, job.live);
                self.running.release(session);
                return Err(e);
            }
        };
        job.turn = turn;
        if let Err(e) = self.relay.watch(viewer, session, Some(turn)) {
            crate::server::platform::log::warn(
                "api",
                format!("session {session}: the compactor's stream did not attach: {e:#}"),
            );
        }
        self.relay.started(session, turn, StartKind::Compact);

        let writer = self.writer.clone();
        let relay = self.relay.clone();
        let sessions = self.sessions.clone();
        let running = self.running.clone();
        if let Err(e) = std::thread::Builder::new()
            .name(format!("compact-{session}"))
            .spawn(move || compact::run(writer, relay, sessions, running, job))
        {
            self.running.release(session);
            return Err(e).context("cannot start the compaction thread");
        }
        Ok(Opened::Started { turn })
    }

    fn open_round(
        &self,
        session: i64,
        stop: std::sync::Arc<AtomicBool>,
        prefix: Option<&str>,
        text: &str,
        suffix: Option<&str>,
    ) -> Result<i64> {
        let conn = Reader::open(&self.db)?;
        let live = self.sessions.take(&self.cfg, &conn, session);
        Reader::close(conn);
        let mut live = live?;
        live.stop = stop.clone();

        let turn = match self.land_user(session, prefix, text, suffix) {
            Ok(turn) => turn,
            Err(e) => {
                self.sessions.put(session, live);
                return Err(e);
            }
        };

        self.spawn_round(session, turn, live, stop)?;
        Ok(turn)
    }

    pub(crate) fn spawn_round(
        &self,
        session: i64,
        turn: i64,
        live: Live,
        _stop: std::sync::Arc<AtomicBool>,
    ) -> Result<()> {
        let preview = round::Preview::new(session, turn, self.relay.clone());
        let writer = self.writer.clone();
        let db = self.db.clone();
        let sessions = self.sessions.clone();
        let running = self.running.clone();
        if let Err(e) = std::thread::Builder::new()
            .name(format!("round-{session}"))
            .spawn(move || round::run(writer, db, sessions, running, session, turn, live, preview))
        {
            self.running.release(session);
            return Err(e).context("cannot start the round thread");
        }
        Ok(())
    }

    fn land_user(
        &self,
        session: i64,
        prefix: Option<&str>,
        text: &str,
        suffix: Option<&str>,
    ) -> Result<i64> {
        let landed = self.writer.push(Submit::UserMessage {
            session: SessionRef::Id(session),
            prefix: prefix.map(str::to_string),
            text: text.to_string(),
            suffix: suffix.map(str::to_string),
            ts: Some(crate::server::platform::time::now_stamp()),
        })?;
        landed.turn.context("the writer opened no round")
    }

    fn plan(
        &self,
        session: i64,
        retention: Retention,
        focus: Option<String>,
        stop: std::sync::Arc<AtomicBool>,
    ) -> Result<Option<compact::Job>> {
        let conn = Reader::open(&self.db)?;
        let out = self.gather(session, retention, focus, stop, &conn);
        Reader::close(conn);
        out
    }

    fn gather(
        &self,
        session: i64,
        retention: Retention,
        focus: Option<String>,
        stop: std::sync::Arc<AtomicBool>,
        conn: &rusqlite::Connection,
    ) -> Result<Option<compact::Job>> {
        let live = self.sessions.take(&self.cfg, conn, session)?;
        let from = call::session::read_start(conn, session)?.unwrap_or(0);
        let turns = compact::turns(conn, session, from)?;
        let plan = cut::plan(
            &turns,
            fold(retention),
            &self.cfg.app.server.request.compact,
        );
        if plan.compacted.is_empty() {
            self.sessions.put(session, live);
            return Ok(None);
        }

        let instruction = self.sessions.instruction(session).unwrap_or_default();
        let provider = provider(conn, session)?;
        let stored: Vec<Message> = turns
            .iter()
            .flat_map(|(_, blocks)| blocks.iter())
            .flat_map(|(_, entries)| entries.iter().cloned())
            .collect();
        let ctx = Wire {
            messages: history::to_wire(&stored, &mut history::Rebuild::default()),
            tools: live.roster.clone(),
        };

        Ok(Some(compact::Job {
            session,
            turn: 0,
            live,
            stop,
            instruction,
            focus: focus.unwrap_or_default(),
            ctx,
            plan,
            provider,
        }))
    }

    pub(crate) fn rewind(&self, target: Target) -> Result<Cut> {
        let cut = post::session::rewind::prune(&self.writer, target)?;
        let conn = Reader::open(&self.db)?;
        let list = call::tool::todo::list(&conn, target.session);
        Reader::close(conn);
        self.sessions.set_todo(target.session, list?);
        Ok(cut)
    }

    pub(crate) fn interrupt(&self, session: i64) {
        if let Some(flag) = self.running.flag(session) {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    pub(crate) fn emit_meta(&self, session: i64) -> Result<()> {
        let conn = Reader::open(&self.db)?;
        let meta = call::server::sessions::meta(&conn, session);
        Reader::close(conn);
        self.relay.session_meta(session, meta?);
        Ok(())
    }

    pub(crate) fn live_cwd(&self, session: i64) -> Result<PathBuf> {
        let conn = Reader::open(&self.db)?;
        let cwd = self.sessions.cwd(&self.cfg, &conn, session);
        Reader::close(conn);
        cwd
    }
}

fn fold(retention: Retention) -> Option<cut::Retention> {
    match retention {
        Retention::Default => None,
        Retention::Tokens(n) => Some(cut::Retention::Tokens(n)),
        Retention::Blocks(n) => Some(cut::Retention::Blocks(n)),
    }
}

fn provider(conn: &rusqlite::Connection, session: i64) -> Result<String> {
    let provider: Option<String> = conn.query_row(
        "SELECT model_provider FROM sessions WHERE id = ?1",
        [session],
        |r| r.get(0),
    )?;
    Ok(provider.unwrap_or_default())
}
