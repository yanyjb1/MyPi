//! The API side of an agent job: run one child session to its end.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Weak;
use std::sync::atomic::AtomicBool;

use anyhow::{Context as _, Result};

use crate::server::agent::r#loop::outlet::{Outlet, ToolEvent};
use crate::server::agent::r#loop::{self as agent_loop, Turn};
use crate::server::agent::request;
use crate::server::agent::request::message::Message;
use crate::server::agent::tools::subagent::{START, Spawn};
use crate::server::config::Config;
use crate::server::storage::{Reader, SessionRef, Submit, Writer};

use super::running::Running;
use super::sessions::{Sessions, SessionsInner};

pub(crate) struct ChildSpawn {
    sessions: Weak<SessionsInner>,
    writer: Writer,
    db: PathBuf,
    cfg: Config,
    running: Running,
}

impl ChildSpawn {
    pub(crate) fn new(
        sessions: Weak<SessionsInner>,
        writer: Writer,
        db: PathBuf,
        cfg: Config,
        running: Running,
    ) -> Self {
        Self {
            sessions,
            writer,
            db,
            cfg,
            running,
        }
    }
}

impl Spawn for ChildSpawn {
    fn run(&self, session: i64, cancel: Arc<AtomicBool>) -> Result<Option<String>> {
        let sessions =
            Sessions::upgrade(self.sessions.clone()).context("the session table is gone")?;
        let stop = self
            .running
            .claim_with(session, cancel)
            .context("the child is already running")?;
        let conn = match Reader::open(&self.db) {
            Ok(conn) => conn,
            Err(e) => {
                self.running.release(session);
                return Err(e);
            }
        };
        let live = sessions.take(&self.cfg, &conn, session);
        Reader::close(conn);
        let mut live = match live {
            Ok(live) => live,
            Err(e) => {
                self.running.release(session);
                return Err(e);
            }
        };
        live.stop = stop.clone();
        let turn = match self.writer.push(Submit::UserMessage {
            session: SessionRef::Id(session),
            prefix: None,
            text: START.to_string(),
            suffix: None,
            ts: Some(crate::server::platform::time::now_stamp()),
        }) {
            Ok(landed) => match landed.turn {
                Some(turn) => turn,
                None => {
                    self.running.release(session);
                    return Err(anyhow::anyhow!("the child round did not open"));
                }
            },
            Err(e) => {
                self.running.release(session);
                return Err(e);
            }
        };
        let result = {
            let conn = match Reader::open(&self.db) {
                Ok(conn) => conn,
                Err(e) => {
                    self.running.release(session);
                    return Err(e);
                }
            };
            let mut silent = Silent;
            let result = agent_loop::run(
                &self.writer,
                &conn,
                &mut live,
                Turn {
                    session,
                    turn_id: turn,
                },
                &mut silent,
            );
            Reader::close(conn);
            result
        };
        self.running.release(session);
        match &result {
            Ok(_) => sessions.put(session, live),
            Err(_) => sessions.drop_live(session),
        }
        result?;
        last_assistant(&self.db, session)
    }
}

struct Silent;

impl Outlet for Silent {
    fn tool_event(&mut self, _event: &ToolEvent) {}
    fn tool_output(&mut self, _call_id: &str, _text: &str) {}
    fn retry(&mut self, _notice: &crate::server::agent::request::retry::RetryNotice) {}
    fn budget_exhausted(&mut self) {}
}

fn last_assistant(db: &std::path::Path, session: i64) -> Result<Option<String>> {
    let conn = Reader::open(db)?;
    let messages = request::rebuild(&conn, session)?;
    Reader::close(conn);
    Ok(messages
        .into_iter()
        .rev()
        .find_map(|message| match message {
            Message::Assistant { content, .. } => Some(content),
            _ => None,
        }))
}
