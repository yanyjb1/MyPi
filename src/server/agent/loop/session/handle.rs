//! One session's handle: its request side, its tool side, and its stop flag.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

use crate::server::agent::request::message::Usage;
use crate::server::agent::request::transport::worker::SessionWorker;
use crate::server::agent::tools::contract::call::Cwd;
use crate::server::agent::tools::contract::env::Env;
use crate::server::platform::stop::Stop;

use super::config::Config;
use super::mailbox::Mailbox;

/// What the running turn has spent so far, and how big the context was on its last lap.
#[derive(Debug, Clone, Default)]
pub struct Partial {
    pub(crate) usage: Usage,
    pub(crate) context: u64,
}

pub(crate) struct Handle {
    pub id: i64,
    /// The request side: the model it asks and the prefix every lap leads with.
    config: Mutex<Config>,
    /// The tool side: this session's conditions, built once when it was booked.
    env: Mutex<Env>,
    /// The one directory the tools and the change door share.
    cwd: Cwd,
    pub mailbox: Arc<Mailbox>,
    pub stop: Arc<Stop>,
    /// This session's HTTP worker: one child, killed and replaced on interrupt.
    pub worker: Arc<SessionWorker>,
    /// This session's history: read once, appended as turns land.
    pub transcript: Arc<super::history::Transcript>,
    pub busy: AtomicBool,
    pub gone: AtomicBool,
    /// One ring per turn that ended: whoever waits for a session to settle waits here.
    settled: (Mutex<u64>, Condvar),
    /// The running turn's partial usage; read only while `busy` holds.
    partial: Mutex<Option<Partial>>,
    /// The session's request archive row.
    meta: Option<i64>,
}

impl Handle {
    pub(crate) fn new(
        birth: super::book::Birth,
        env: Env,
        transcript: Arc<super::history::Transcript>,
    ) -> Self {
        let config = Config::new(&birth);
        Self {
            id: birth.session,
            config: Mutex::new(config),
            cwd: env.workspace.shared(),
            stop: env.stop.clone(),
            env: Mutex::new(env),
            mailbox: Arc::new(Mailbox::new()),
            worker: SessionWorker::new(),
            transcript,
            busy: AtomicBool::new(false),
            gone: AtomicBool::new(false),
            settled: (Mutex::new(0), Condvar::new()),
            partial: Mutex::new(None),
            meta: birth.meta,
        }
    }

    /// The request side, as it stands now.
    pub(crate) fn config(&self) -> Config {
        self.config
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub(crate) fn config_mut(&self) -> MutexGuard<'_, Config> {
        self.config.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The tool side, as it stands now.
    pub(crate) fn env(&self) -> MutexGuard<'_, Env> {
        self.env.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The turn is over: the flag drops, the idle waiters hear it, the scheduler wakes.
    pub(crate) fn mark_idle(&self) {
        self.busy.store(false, Ordering::Release);
        self.ring_settle();
        self.mailbox.notify();
    }

    /// Wake whoever waits for a session to settle.
    pub(crate) fn ring_settle(&self) {
        let mut generation = self.settled.0.lock().unwrap_or_else(|e| e.into_inner());
        *generation += 1;
        self.settled.1.notify_all();
    }

    /// Block until no turn runs on this session.
    pub(crate) fn wait_idle(&self) {
        let mut generation = self.settled.0.lock().unwrap_or_else(|e| e.into_inner());
        while self.busy.load(Ordering::Acquire) {
            generation = self
                .settled
                .1
                .wait(generation)
                .unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Wait at most `wait` for the session to settle.
    pub(crate) fn wait_idle_for(&self, wait: std::time::Duration) {
        let generation = self.settled.0.lock().unwrap_or_else(|e| e.into_inner());
        if self.busy.load(Ordering::Acquire) {
            let _ = self
                .settled
                .1
                .wait_timeout(generation, wait)
                .unwrap_or_else(|e| e.into_inner());
        }
    }

    /// This session's history cache.
    pub(crate) fn transcript(&self) -> &Arc<super::history::Transcript> {
        &self.transcript
    }

    /// One lap landed: the running turn's tally so far, and its context size.
    pub(crate) fn publish(&self, usage: &Usage, context: u64) {
        let mut slot = self.partial.lock().unwrap_or_else(|e| e.into_inner());
        *slot = Some(Partial {
            usage: *usage,
            context,
        });
    }

    /// The running turn's partial usage; nothing while the session sits idle.
    pub(crate) fn partial(&self) -> Option<Partial> {
        if !self.busy.load(std::sync::atomic::Ordering::Acquire) {
            return None;
        }
        self.partial
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// A turn is opening: the tally of the one before it is spent.
    pub(crate) fn clear_partial(&self) {
        *self.partial.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    /// The model the next request would use.
    pub(crate) fn model(&self) -> (String, String) {
        let config = self.config.lock().unwrap_or_else(|e| e.into_inner());
        (config.model.provider.clone(), config.model.model.clone())
    }

    /// Where the tools stand now.
    pub(crate) fn cwd(&self) -> PathBuf {
        self.cwd.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Move the session's directory; the tools read it on their next call.
    pub(crate) fn chdir(&self, real: PathBuf) {
        *self.cwd.write().unwrap_or_else(|e| e.into_inner()) = real;
    }

    pub(crate) fn interrupt(&self) {
        if !self.busy.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        self.stop.set();
        self.worker.kill();
        self.mailbox.notify();
    }

    pub(crate) fn meta(&self) -> Option<i64> {
        self.meta
    }
}
