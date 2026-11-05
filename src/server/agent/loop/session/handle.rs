//! One session's handle: its birth values, its live state, its stop flag.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::server::agent::request::transport::worker::SessionWorker;

use super::book::Birth;
use super::live::Live;
use super::mailbox::Mailbox;

pub(crate) struct Handle {
    pub id: i64,
    pub birth: Mutex<Birth>,
    pub mailbox: Arc<Mailbox>,
    pub stop: Arc<AtomicBool>,
    /// This session's HTTP worker: one child, killed and replaced on interrupt.
    pub worker: Arc<SessionWorker>,
    pub busy: AtomicBool,
    pub gone: AtomicBool,
    pub live: Mutex<Option<Live>>,
}

impl Handle {
    pub(crate) fn new(birth: Birth) -> Self {
        Self {
            id: birth.session,
            birth: Mutex::new(birth),
            mailbox: Arc::new(Mailbox::new()),
            stop: Arc::new(AtomicBool::new(false)),
            worker: SessionWorker::new(),
            busy: AtomicBool::new(false),
            gone: AtomicBool::new(false),
            live: Mutex::new(None),
        }
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
        if self.busy.load(std::sync::atomic::Ordering::Acquire) {
            self.stop.store(true, std::sync::atomic::Ordering::Release);
        }
        self.worker.interrupt();
        self.mailbox.notify();
    }

    pub(crate) fn meta(&self) -> Option<i64> {
        self.birth().meta
    }
}
