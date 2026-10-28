//! Which sessions are mid-round, and the flag that stops them.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};

/// The running rounds, keyed by session.
#[derive(Clone, Default)]
pub(crate) struct Running {
    flags: Arc<Mutex<HashMap<i64, Arc<AtomicBool>>>>,
}

impl Running {
    /// Claim the session; `None` = it is already mid-round.
    pub(crate) fn claim(&self, session: i64) -> Option<Arc<AtomicBool>> {
        let mut flags = self.flags();
        if flags.contains_key(&session) {
            return None;
        }
        let flag = Arc::new(AtomicBool::new(false));
        flags.insert(session, flag.clone());
        Some(flag)
    }

    /// Claim the session with a caller-owned stop flag.
    pub(crate) fn claim_with(
        &self,
        session: i64,
        flag: Arc<AtomicBool>,
    ) -> Option<Arc<AtomicBool>> {
        let mut flags = self.flags();
        if flags.contains_key(&session) {
            return None;
        }
        flags.insert(session, flag.clone());
        Some(flag)
    }

    /// The round is over.
    pub(crate) fn release(&self, session: i64) {
        self.flags().remove(&session);
    }

    /// The running round's stop flag.
    pub(crate) fn flag(&self, session: i64) -> Option<Arc<AtomicBool>> {
        self.flags().get(&session).cloned()
    }

    /// Is anything running at all?
    pub(crate) fn is_empty(&self) -> bool {
        self.flags().is_empty()
    }

    fn flags(&self) -> MutexGuard<'_, HashMap<i64, Arc<AtomicBool>>> {
        self.flags.lock().unwrap_or_else(|e| e.into_inner())
    }
}
