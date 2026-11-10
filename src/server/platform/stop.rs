//! One stop signal: a flag, and the actions that abort the work waiting under it.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

type Action = Arc<dyn Fn() + Send + Sync>;

struct Slot {
    id: u64,
    action: Action,
}

pub struct Stop {
    stopped: AtomicBool,
    next: AtomicU64,
    actions: Mutex<Vec<Slot>>,
}

/// One registration; dropping it takes the action off the signal.
pub struct Hook {
    stop: Weak<Stop>,
    id: u64,
}

impl Stop {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            stopped: AtomicBool::new(false),
            next: AtomicU64::new(0),
            actions: Mutex::new(Vec::new()),
        })
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    /// Set the flag, then run every registered action once on this thread.
    pub fn set(&self) {
        let mut actions = self.lock();
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        let taken = std::mem::take(&mut *actions);
        drop(actions);
        for slot in taken {
            (slot.action)();
        }
    }

    /// Clear the flag; the table must already be empty.
    pub fn reset(&self) {
        self.stopped.store(false, Ordering::Release);
    }

    /// Register an action; a signal that already fired runs it here and now.
    pub fn on_stop(self: &Arc<Self>, action: impl Fn() + Send + Sync + 'static) -> Hook {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let action: Action = Arc::new(action);
        let mut actions = self.lock();
        if self.stopped.load(Ordering::Acquire) {
            drop(actions);
            action();
            return Hook {
                stop: Arc::downgrade(self),
                id,
            };
        }
        actions.push(Slot { id, action });
        drop(actions);
        Hook {
            stop: Arc::downgrade(self),
            id,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Slot>> {
        self.actions.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Drop for Hook {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.upgrade() {
            stop.lock().retain(|slot| slot.id != self.id);
        }
    }
}
