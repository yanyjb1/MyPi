//! The API side of a wake: a channel, one worker, and a round that has no user message.

use anyhow::{Context, Result};

use super::Driver;
use crate::server::agent::tools::jobs::{Wake, WakeSink};
use crate::server::api::subscribe::event::StartKind;
use crate::server::storage::{Reader, Submit};

pub(crate) struct ChannelSink(pub std::sync::mpsc::Sender<Wake>);

impl WakeSink for ChannelSink {
    fn wake(&self, wake: Wake) {
        let _ = self.0.send(wake);
    }
}

pub(crate) fn spawn_worker(rx: std::sync::mpsc::Receiver<Wake>) {
    std::thread::Builder::new()
        .name("api-wake".to_string())
        .spawn(move || {
            for wake in rx {
                let Some(state) = super::state_ready() else {
                    continue;
                };
                let driver = state.lock().unwrap_or_else(|e| e.into_inner());
                if let Err(e) = driver.wake(wake) {
                    crate::server::platform::log::warn("api", format!("wake failed: {e:#}"));
                }
            }
        })
        .expect("cannot start api-wake");
}

impl Driver {
    pub(crate) fn wake(&self, wake: Wake) -> Result<()> {
        if !self.relay.watched(wake.session) {
            self.sessions.cancel_jobs(wake.session);
            return Ok(());
        }
        if self.running.flag(wake.session).is_some() {
            self.queue_wake(wake.session, wake.text);
            return Ok(());
        }
        self.open_wake(wake.session, wake.text)
    }

    fn open_wake(&self, session: i64, text: String) -> Result<()> {
        let Some(stop) = self.running.claim(session) else {
            self.queue_wake(session, text);
            return Ok(());
        };
        let conn = match Reader::open(&self.db) {
            Ok(conn) => conn,
            Err(e) => {
                self.running.release(session);
                return Err(e);
            }
        };
        let live = self.sessions.take(&self.cfg, &conn, session);
        Reader::close(conn);
        let mut live = match live {
            Ok(live) => live,
            Err(e) => {
                self.running.release(session);
                return Err(e);
            }
        };
        live.stop = stop.clone();

        let turn = match self.land_wake(session, text) {
            Ok(turn) => turn,
            Err(e) => {
                self.sessions.put(session, live);
                self.running.release(session);
                return Err(e);
            }
        };
        self.relay.started(session, turn, StartKind::Wake);
        self.spawn_round(session, turn, live, stop)
    }

    fn queue_wake(&self, session: i64, text: String) {
        if let Some(queue) = self.sessions.wake_queue(session) {
            queue.lock().unwrap_or_else(|e| e.into_inner()).push(text);
        }
    }

    fn land_wake(&self, session: i64, text: String) -> Result<i64> {
        let landed = self.writer.push(Submit::Wake {
            session,
            text,
            ts: Some(crate::server::platform::time::now_stamp()),
        })?;
        landed.turn.context("the writer opened no round")
    }
}
