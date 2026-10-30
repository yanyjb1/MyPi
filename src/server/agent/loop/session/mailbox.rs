//! The session's inbound line: two queues, one mechanism.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};

use super::book::Message;

struct Inner {
    wake: VecDeque<String>,
    user: VecDeque<Message>,
}

/// Two queues: deterministic wake text, and user messages that may be changed
/// or taken back before the loop reads them.
pub struct Mailbox {
    inner: Mutex<Inner>,
    cv: Condvar,
}

impl Mailbox {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                wake: VecDeque::new(),
                user: VecDeque::new(),
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

    pub fn push_user(&self, msg: Message) {
        self.lock().user.push_back(msg);
        self.cv.notify_all();
    }

    pub fn take_wake(&self) -> Option<String> {
        self.lock().wake.pop_front()
    }

    pub fn take_user(&self) -> Option<Message> {
        self.lock().user.pop_front()
    }

    pub fn has_any(&self) -> bool {
        let g = self.lock();
        !g.wake.is_empty() || !g.user.is_empty()
    }

    pub fn is_empty(&self) -> bool {
        !self.has_any()
    }

    pub fn clear_user(&self) {
        self.lock().user.clear();
    }

    /// Wait until either queue has something or the session is gone.
    pub fn wait_any(&self, gone: &AtomicBool) {
        let mut g = self.lock();
        while g.wake.is_empty() && g.user.is_empty() && !gone.load(Ordering::Acquire) {
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
