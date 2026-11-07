//! One session's handle: its birth values, its live state, its stop flag.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::server::agent::request::message::Usage;
use crate::server::agent::request::transport::worker::SessionWorker;

use super::book::Birth;
use super::live::Live;
use super::mailbox::Mailbox;

/// What the running turn has spent so far, and how big the context was on its last lap.
#[derive(Debug, Clone, Default)]
pub struct Partial {
    pub(crate) usage: Usage,
    pub(crate) context: u64,
}

pub(crate) struct Handle {
    pub id: i64,
    pub birth: Mutex<Birth>,
    pub mailbox: Arc<Mailbox>,
    pub stop: Arc<AtomicBool>,
    /// This session's HTTP worker: one child, killed and replaced on interrupt.
    pub worker: Arc<SessionWorker>,
    /// This session's history: read once, appended as turns land.
    pub transcript: Arc<super::history::Transcript>,
    pub busy: AtomicBool,
    pub gone: AtomicBool,
    pub live: Mutex<Option<Live>>,
    /// The running turn's partial usage; read only while `busy` holds.
    partial: Mutex<Option<Partial>>,
}

impl Handle {
    pub(crate) fn new(birth: Birth, transcript: Arc<super::history::Transcript>) -> Self {
        Self {
            id: birth.session,
            birth: Mutex::new(birth),
            mailbox: Arc::new(Mailbox::new()),
            stop: Arc::new(AtomicBool::new(false)),
            worker: SessionWorker::new(),
            transcript,
            busy: AtomicBool::new(false),
            gone: AtomicBool::new(false),
            live: Mutex::new(None),
            partial: Mutex::new(None),
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

    /// The model the next request would use, without copying the whole birth.
    pub(crate) fn model(&self) -> (String, String) {
        let birth = self.birth.lock().unwrap_or_else(|e| e.into_inner());
        (birth.provider.clone(), birth.model.clone())
    }

    pub(crate) fn birth(&self) -> Birth {
        self.birth.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(crate) fn take_live(&self, dir: &std::path::Path) -> Live {
        if let Some(live) = self.live.lock().unwrap_or_else(|e| e.into_inner()).take() {
            return live;
        }
        let birth = self.birth();
        Live::new(&birth, dir)
    }

    pub(crate) fn put_live(&self, live: Live) {
        *self.live.lock().unwrap_or_else(|e| e.into_inner()) = Some(live);
    }

    pub(crate) fn interrupt(&self) {
        if !self.busy.load(std::sync::atomic::Ordering::Acquire) {
            return;
        }
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        self.worker.interrupt();
        self.mailbox.notify();
    }

    pub(crate) fn meta(&self) -> Option<i64> {
        self.birth().meta
    }
}
