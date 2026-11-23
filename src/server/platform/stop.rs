// One stop signal: a flag, and the actions that abort the work waiting under it.

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
    running: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

// One registration; dropping it takes the action off the signal.
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
            running: Mutex::new(Vec::new()),
        })
    }

    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }

    // Set the flag, then run every registered action once, one thread each.
    pub fn set(&self) {
        let mut actions = self.lock();
        if self.stopped.swap(true, Ordering::AcqRel) {
            return;
        }
        let taken = std::mem::take(&mut *actions);
        drop(actions);
        let mut running = self.running();
        running.clear();
        for slot in taken {
            let inline = slot.action.clone();
            let spawned = std::thread::Builder::new()
                .name(format!("mypi-stop-{}", slot.id))
                .spawn(move || (slot.action)());
            match spawned {
                Ok(handle) => running.push(handle),
                Err(_) => inline(),
            }
        }
    }

    // Wait for the actions the last `set` dispatched. A stop returns at once; this is for a caller that is about to touch what those actions are taking down.
    pub fn settle(&self) {
        let handles = std::mem::take(&mut *self.running());
        for handle in handles {
            let _ = handle.join();
        }
    }

    // Clear the flag; the table must already be empty.
    pub fn reset(&self) {
        self.stopped.store(false, Ordering::Release);
    }

    // Sleep this long unless the turn stops first; `false` means a stop woke it, so nothing that would follow should follow. The bell is a channel: a stop that landed before the wait still wakes it.
    pub fn pause(self: &Arc<Self>, dur: std::time::Duration) -> bool {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let _hook = self.on_stop(move || {
            let _ = tx.send(());
        });
        let woken = match rx.recv_timeout(dur) {
            Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => true,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => false,
        };
        !(woken || self.is_stopped())
    }

    // Register an action; a signal that already fired runs it here and now.
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

    fn running(&self) -> MutexGuard<'_, Vec<std::thread::JoinHandle<()>>> {
        self.running.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Drop for Hook {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.upgrade() {
            stop.lock().retain(|slot| slot.id != self.id);
        }
    }
}
