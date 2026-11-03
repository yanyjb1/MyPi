//! The session's inbound: one wake queue, and one user slot the front end owns.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};

use super::book::Message;

struct Inner {
    wake: VecDeque<String>,
    send: Option<Message>,
}

/// The wake queue and the one user message a front end may rewrite until it is taken.
pub struct Mailbox {
    inner: Mutex<Inner>,
    cv: Condvar,
}

impl Mailbox {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                wake: VecDeque::new(),
                send: None,
            }),
            cv: Condvar::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn push_wake(&self, text: String) {
        self.lock().wake.push_back(text);
        self.cv.notify_all();
    }

    /// Overwrite the slot whole; a message always carries a body.
    pub fn write(&self, msg: Message) {
        self.lock().send = Some(msg);
        self.cv.notify_all();
    }

    /// The front end takes its message back.
    pub fn del(&self) {
        self.lock().send = None;
    }

    pub fn take_wake(&self) -> Option<String> {
        self.lock().wake.pop_front()
    }

    /// One message; taking the slot is clearing it.
    pub fn take_send(&self) -> Option<Message> {
        self.lock().send.take()
    }

    pub fn has_any(&self) -> bool {
        let g = self.lock();
        !g.wake.is_empty() || g.send.is_some()
    }

    /// Wait until either the queue or the slot has something, or the session is gone.
    pub fn wait_any(&self, gone: &AtomicBool) {
        let mut g = self.lock();
        while g.wake.is_empty() && g.send.is_none() && !gone.load(Ordering::Acquire) {
            g = self.cv.wait(g).unwrap_or_else(|e| e.into_inner());
        }
    }

    pub fn notify(&self) {
        self.cv.notify_all();
    }
}

impl Default for Mailbox {
    fn default() -> Self {
        Self::new()
    }
}
