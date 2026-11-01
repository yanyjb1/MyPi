//! The job service, one per session, the session table it wakes, and its numbers from config.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::handle::Handle;
use crate::server::agent::r#loop::job;
use crate::server::agent::r#loop::job::{Wake, WakeSink};
use crate::server::config::{
    Config, KEY_CONFIG_SERVER_AGENT_TOOLS_KAIROS_MAX_CONCURRENT,
    KEY_CONFIG_SERVER_AGENT_TOOLS_KAIROS_MAX_OUTPUT_BYTES,
    KEY_CONFIG_SERVER_AGENT_TOOLS_KAIROS_MAX_RETAINED,
    KEY_CONFIG_SERVER_AGENT_TOOLS_KAIROS_TIMEOUT_SECS,
};

/// The loop's open sessions; a finished job's wake goes to its session's mailbox.
pub(crate) struct Slots {
    inner: Arc<Mutex<HashMap<i64, Arc<Handle>>>>,
}

impl Slots {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub(crate) fn get(&self, session: i64) -> Option<Arc<Handle>> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&session)
            .cloned()
    }

    /// Take one slot; `false` means the session is already booked.
    pub(crate) fn insert(&self, session: i64, handle: Arc<Handle>) -> bool {
        let mut slots = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if slots.contains_key(&session) {
            return false;
        }
        slots.insert(session, handle);
        true
    }

    pub(crate) fn remove(&self, session: i64) -> Option<Arc<Handle>> {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&session)
    }
}

impl WakeSink for Slots {
    fn wake(&self, wake: Wake) {
        if let Some(handle) = self.get(wake.session) {
            handle.mailbox.push_wake(wake.text);
        }
    }
}

/// The scheduler's numbers, as the config hands them over.
pub(crate) fn values(cfg: &Config) -> job::Values {
    let int = |key: &str, fallback: i64| -> u64 {
        cfg.get(key, None)
            .ok()
            .and_then(|value| value.as_int())
            .unwrap_or(fallback)
            .max(0) as u64
    };
    job::Values {
        max_concurrent: int(KEY_CONFIG_SERVER_AGENT_TOOLS_KAIROS_MAX_CONCURRENT, 4).max(1) as usize,
        timeout_secs: int(KEY_CONFIG_SERVER_AGENT_TOOLS_KAIROS_TIMEOUT_SECS, 3600).max(1),
        max_output_bytes: int(
            KEY_CONFIG_SERVER_AGENT_TOOLS_KAIROS_MAX_OUTPUT_BYTES,
            8_388_608,
        ) as usize,
        max_retained: int(KEY_CONFIG_SERVER_AGENT_TOOLS_KAIROS_MAX_RETAINED, 8).max(1) as usize,
    }
}
