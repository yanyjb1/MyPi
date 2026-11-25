// The conversation table: which session id is held, and letting go of the ones gone quiet.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::ipc::endpoint::Endpoint;

use super::held::Held;

// The bridge's whole table: a session id, and the connection holding it.
pub struct Hub {
    endpoint: Endpoint,
    held: Mutex<BTreeMap<i64, Arc<Held>>>,
    bare: Mutex<Option<Arc<Held>>>,
}

impl Hub {
    pub fn new(endpoint: Endpoint) -> Hub {
        Hub {
            endpoint,
            held: Mutex::new(BTreeMap::new()),
            bare: Mutex::new(None),
        }
    }

    // The connection that follows nothing: questions naming no session use it, and it keeps the server up.
    pub fn bare(&self) -> Result<Arc<Held>, String> {
        let mut slot = self.bare.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(held) = slot.as_ref()
            && !held.is_dead()
        {
            return Ok(Arc::clone(held));
        }
        let held = Arc::new(Held::open(&self.endpoint, None)?);
        let thread = Arc::clone(&held);
        std::thread::Builder::new()
            .name("bridge-bare".to_string())
            .spawn(move || thread.read())
            .map_err(|e| e.to_string())?;
        *slot = Some(Arc::clone(&held));
        Ok(held)
    }

    // The connection holding one conversation, opened the first time something asks for it.
    pub fn get(&self, session: i64) -> Result<Arc<Held>, String> {
        let mut table = self.held.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(held) = table.get(&session)
            && !held.is_dead()
        {
            return Ok(Arc::clone(held));
        }
        table.remove(&session);
        let held = Arc::new(Held::open(&self.endpoint, Some(session))?);
        let thread = Arc::clone(&held);
        std::thread::Builder::new()
            .name(format!("bridge-session-{session}"))
            .spawn(move || thread.read())
            .map_err(|e| e.to_string())?;
        table.insert(session, Arc::clone(&held));
        Ok(held)
    }

    // Let one conversation go: its stream, its booking and its browser tabs go back to the server.
    fn release(&self, session: i64) {
        let held = self
            .held
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&session);
        if let Some(held) = held {
            held.close();
        }
    }

    // Drop every conversation nobody has touched for a while and that is running nothing.
    pub fn sweep(&self, idle: Duration) {
        let stale: Vec<i64> = {
            let table = self.held.lock().unwrap_or_else(|e| e.into_inner());
            table
                .iter()
                .filter(|(_, held)| held.is_stale(idle))
                .map(|(session, _)| *session)
                .collect()
        };
        for session in stale {
            self.release(session);
        }
    }

    pub fn stop(&self) {
        let held: Vec<Arc<Held>> = {
            let mut table = self.held.lock().unwrap_or_else(|e| e.into_inner());
            std::mem::take(&mut *table)
                .into_values()
                .collect::<Vec<_>>()
        };
        for held in held {
            held.close();
        }
        if let Some(bare) = self.bare.lock().unwrap_or_else(|e| e.into_inner()).take() {
            bare.close();
        }
    }
}
