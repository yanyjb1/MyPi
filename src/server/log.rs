//! In-memory diagnostics log — for things that happened to **us**, not to the
//! conversation.
//!
//! Retries, timeouts, dropped connections, "gateway said 503, trying again":
//! all of it is local narration about our own behavior. It never enters a
//! request, so it is never persisted (see the storage contract in
//! `DELETED.md`): the database holds what the model saw, this holds what we
//! did about it. A restart loses it, which is the point.
//!
//! Process-wide and bounded: turn threads log from wherever they run, and a
//! long-lived process must not grow without bound. Readers (a TUI pane, a
//! bridge, a test) snapshot the recent tail.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// How loud a record is. Deliberately small: this is an operator log, not a
/// framework.
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

/// The bounded buffer itself, separated from the global so it can be tested
/// without every test in the crate fighting over one shared ring.
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
    ring().lock().expect("日志锁中毒").push(rec, CAPACITY);
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
    ring().lock().expect("日志锁中毒").recent(limit)
}

/// The records the `scope` produced, newest last. Lets a caller (a UI pane, a
/// test) read one subject's history out of a shared ring.
pub fn by_scope(scope: &str) -> Vec<Record> {
    ring()
        .lock()
        .expect("日志锁中毒")
        .recs
        .iter()
        .filter(|r| r.scope == scope)
        .cloned()
        .collect()
}

/// How many records we currently hold (tests and a statusline counter).
pub fn len() -> usize {
    ring().lock().expect("日志锁中毒").recs.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(text: &str) -> Record {
        Record {
            at_ms: 0,
            level: Level::Info,
            scope: "test".into(),
            text: text.into(),
        }
    }

    #[test]
    fn the_ring_keeps_the_newest_and_never_grows_past_its_cap() {
        // 日志是给「刚刚发生了什么」用的：满了就丢最老的，不许无限长。
        // （测 Ring 本身，不碰全局，免得和并行跑的用例抢一个环。）
        let mut ring = Ring::new(4);
        for i in 0..7 {
            ring.push(rec(&format!("第 {i} 行")), 4);
        }
        let all = ring.recent(0);
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].text, "第 3 行");
        assert_eq!(all[3].text, "第 6 行");
        let tail = ring.recent(2);
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].text, "第 5 行");
    }
}
