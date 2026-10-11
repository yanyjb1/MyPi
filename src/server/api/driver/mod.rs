//! The driver: the state every door is reached through — the writer, one `Live` per session, the
//! running rounds, the relay, the config. `state` hands out the one instance the api shares.

mod compact;
mod relay;
mod round;
mod running;
mod sessions;

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Receiver;
use std::sync::{Mutex, OnceLock};

use crate::server::agent::request::assembly::compact as cut;
use crate::server::agent::request::assembly::history;
use crate::server::agent::request::message::{Context as Wire, Message};
use crate::server::api::call;
use crate::server::api::post;
use crate::server::api::reply::stream::StreamItem;
use crate::server::api::subscribe::compaction::{CompactOp, Retention};
use crate::server::api::subscribe::submit::SubmitOp;
use crate::server::api::subscribe::Opened;
use crate::server::api::types::{Call, Reply, Viewer};
use crate::server::config::Config;
use crate::server::storage::{Cut, Reader, SessionRef, Submit, Target, Writer};

use relay::Relay;
use running::Running;
use sessions::Sessions;

/// The api's state, built the first time a door needs it.
static STATE: OnceLock<Mutex<Driver>> = OnceLock::new();

/// The guard around that build, so two callers cannot both open the database.
static BUILDING: Mutex<()> = Mutex::new(());

/// The api's state, opening the database on the first call. The file is
/// [`crate::server::storage::default_path`]; `batch` comes from the config.
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
    /// Open the database, start the writer thread, and take the api's relay
    /// port.
    fn open(cfg: Config, db: &Path) -> Result<Driver> {
        let writer = Writer::open(db, cfg.app.server.storage.batch_tools)?;
        let relay = Relay::start(&writer)?;
        Ok(Driver {
            cfg,
            db: db.to_path_buf(),
            writer,
            sessions: Sessions::default(),
            running: Running::default(),
            relay,
        })
    }

    /// Watch one session: every piece about it arrives on the returned channel
    /// until this viewer stops watching.
    pub(crate) fn watch(&self, viewer: Viewer, session: i64) -> Receiver<StreamItem> {
        self.relay.attach(viewer, session)
    }

    /// Stop watching one session.
    pub(crate) fn unwatch(&self, viewer: Viewer, session: i64) {
        self.relay.detach(viewer, session);
        self.reap(session);
    }

    /// Let a session's `Live` go when nobody watches it and nothing runs in it. Everything that
    /// matters is in storage, so the next round builds the state again.
    pub(crate) fn reap(&self, session: i64) {
        if self.running.flag(session).is_none() && !self.relay.watched(session) {
            self.sessions.drop_live(session);
        }
    }

    /// Answer one question: open a read connection, answer it, close it.
    pub(crate) fn call(&self, call: Call) -> Result<Reply> {
        // The working directory in memory is the one value a row cannot answer:
        // only the driver holds it.
        let cwd = match &call {
            Call::Meta { session } => {
                let conn = Reader::open(&self.db)?;
                let cwd = self.sessions.cwd(&self.cfg, &conn, *session);
                Reader::close(conn);
                cwd?
            }
            _ => PathBuf::new(),
        };
        let conn = Reader::open(&self.db)?;
        let out = call::dispatch(&self.cfg, &conn, &cwd, call);
        Reader::close(conn);
        out
    }

    /// Cut a session's list back to the picked block. The caller opens the next
    /// round on what survived.
    pub(crate) fn rewind(&mut self, target: Target) -> Result<Cut> {
        let session = target.session;
        let cut = post::rewind::rewind(&self.writer, target)?;
        // The cut may have taken the rounds the todo list was built from.
        let conn = Reader::open(&self.db)?;
        let list = call::todo::list(&conn, session);
        Reader::close(conn);
        self.sessions.set_todo(session, list?);
        Ok(cut)
    }

    /// Ask the running round of one session to stop. Nothing running is not an error.
    pub(crate) fn interrupt(&self, session: i64) {
        if let Some(flag) = self.running.flag(session) {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    /// Land the user's message and put the round on a thread of its own. The round id goes back
    /// the moment it exists, before the thread does anything.
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

        // The user's message lands first: from here on the round exists, and
        // the loop reads it back out of the list.
        let landed = self.writer.push(Submit::UserMessage {
            session: SessionRef::Id(session),
            prefix: prefix.map(str::to_string),
            text: text.to_string(),
            suffix: suffix.map(str::to_string),
            ts: Some(crate::server::platform::time::now_stamp()),
        })?;
        let turn = landed.turn.context("the writer opened no round")?;

        let preview = round::Preview::new(session, stop, self.writer.clone(), self.relay.clone());
        let writer = self.writer.clone();
        let db = self.db.clone();
        let sessions = self.sessions.clone();
        let running = self.running.clone();
        std::thread::Builder::new()
            .name(format!("round-{session}"))
            .spawn(move || {
                round::run(
                    writer, db, sessions, running, session, turn, live, preview,
                )
            })
            .context("cannot start the round thread")?;
        Ok(turn)
    }

    /// Gather everything one compaction needs (`None` = there is nothing to
    /// compact).
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

    /// The gathering itself, on a connection the caller owns.
    fn gather(
        &self,
        session: i64,
        retention: Retention,
        focus: Option<String>,
        stop: std::sync::Arc<AtomicBool>,
        conn: &rusqlite::Connection,
    ) -> Result<Option<compact::Job>> {
        let live = self.sessions.take(&self.cfg, conn, session)?;
        let from = call::read_start(conn, session)?.unwrap_or(0);
        let turns = compact::turns(conn, session, from)?;
        let plan = cut::plan(
            &turns,
            fold(retention),
            &self.cfg.app.server.request.compact,
        );
        if plan.compacted.is_empty() {
            // Nothing to summarize: give the session back and refuse.
            self.sessions.put(session, live);
            return Ok(None);
        }

        let instruction = self.sessions.instruction(session).unwrap_or_default();
        let provider = provider(conn, session)?;
        // The summarization request replays the live context; the system prompt goes on at the
        // assembly layer, like every request.
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
            live,
            stop,
            instruction,
            focus: focus.unwrap_or_default(),
            ctx,
            plan,
            provider,
        }))
    }
}

impl SubmitOp for Driver {
    fn submit(
        &mut self,
        session: i64,
        prefix: Option<&str>,
        text: &str,
        suffix: Option<&str>,
    ) -> Result<Opened> {
        let Some(stop) = self.running.claim(session) else {
            return Ok(Opened::Busy);
        };
        // The round's pieces are not handed out here: they go to whoever
        // watches the session (see `watch`), which a front end does first.
        match self.open_round(session, stop, prefix, text, suffix) {
            Ok(turn) => Ok(Opened::Started { turn: Some(turn) }),
            Err(e) => {
                self.running.release(session);
                Err(e)
            }
        }
    }
}

impl CompactOp for Driver {
    fn compact(
        &mut self,
        session: i64,
        retention: Retention,
        focus: Option<String>,
    ) -> Result<Opened> {
        let Some(stop) = self.running.claim(session) else {
            return Ok(Opened::Busy);
        };
        let job = match self.plan(session, retention, focus, stop) {
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
        // The compaction's round id is opened inside its own thread: it arrives with `Ended`.
        Ok(Opened::Started { turn: None })
    }
}

/// The api's retention, as the compaction plan wants it (`None` = the config's
/// tail budget).
fn fold(retention: Retention) -> Option<cut::Retention> {
    match retention {
        Retention::Default => None,
        Retention::Tokens(n) => Some(cut::Retention::Tokens(n)),
        Retention::Blocks(n) => Some(cut::Retention::Blocks(n)),
    }
}

/// The provider this session is on (the archive records it).
fn provider(conn: &rusqlite::Connection, session: i64) -> Result<String> {
    let provider: Option<String> = conn.query_row(
        "SELECT model_provider FROM sessions WHERE id = ?1",
        [session],
        |r| r.get(0),
    )?;
    Ok(provider.unwrap_or_default())
}
