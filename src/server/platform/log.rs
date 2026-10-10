//! In-memory diagnostics: what happened to **us** — retries, timeouts, dropped connections.
//! Nothing here ever enters a request.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// How loud a record is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

/// One line of local narration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// Milliseconds since the Unix epoch (seconds-resolution clocks are
    /// useless when diagnosing a retry storm).
    pub at_ms: u128,
    pub level: Level,
    /// Where it came from — usually the model id or a session id.
    pub scope: String,
    pub text: String,
}

// Ring capacity. 256 lines is several sessions' worth of retry noise and a few
// KB of RAM; anything older is not worth a scrollback.
const CAPACITY: usize = 256;

/// The bounded buffer, kept apart from the global so the capacity is a
/// parameter rather than a constant baked into the type.
#[derive(Debug)]
struct Ring {
    recs: VecDeque<Record>,
}

impl Ring {
    fn new(capacity: usize) -> Self {
        Self {
            recs: VecDeque::with_capacity(capacity),
        }
    }

    fn push(&mut self, rec: Record, capacity: usize) {
        if self.recs.len() == capacity {
            self.recs.pop_front();
        }
        self.recs.push_back(rec);
    }

    fn recent(&self, limit: usize) -> Vec<Record> {
        let skip = if limit == 0 || self.recs.len() <= limit {
            0
        } else {
            self.recs.len() - limit
        };
        self.recs.iter().skip(skip).cloned().collect()
    }
}

static LOG: OnceLock<Mutex<Ring>> = OnceLock::new();

fn ring() -> &'static Mutex<Ring> {
    LOG.get_or_init(|| Mutex::new(Ring::new(CAPACITY)))
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

/// Append one record, dropping the oldest when full.
pub fn push(level: Level, scope: impl Into<String>, text: impl Into<String>) {
    let rec = Record {
        at_ms: now_ms(),
        level,
        scope: scope.into(),
        text: text.into(),
    };
    ring().lock().expect("log lock poisoned").push(rec, CAPACITY);
}

pub fn info(scope: impl Into<String>, text: impl Into<String>) {
    push(Level::Info, scope, text);
}

pub fn warn(scope: impl Into<String>, text: impl Into<String>) {
    push(Level::Warn, scope, text);
}

pub fn error(scope: impl Into<String>, text: impl Into<String>) {
    push(Level::Error, scope, text);
}

/// The most recent records, oldest first. `limit` = 0 means "everything held".
pub fn recent(limit: usize) -> Vec<Record> {
    ring().lock().expect("log lock poisoned").recent(limit)
}

/// The records one `scope` produced, newest last.
pub fn by_scope(scope: &str) -> Vec<Record> {
    ring()
        .lock()
        .expect("log lock poisoned")
        .recs
        .iter()
        .filter(|r| r.scope == scope)
        .cloned()
        .collect()
}

/// How many records we currently hold.
pub fn len() -> usize {
    ring().lock().expect("log lock poisoned").recs.len()
}
