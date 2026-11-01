//! Rebuild a session's list from its recorded `todo` calls. Pure: the same calls give the same list.

use anyhow::Result;

use super::ops::{self, Call};
use super::state::TodoItem;
use super::tool::{PARAMS, RESET};
use crate::server::agent::tools::contract::view::CallRecord;
use crate::server::agent::tools::contract::view::Source;
use crate::server::agent::tools::spec::Args;

/// The list a sequence of calls adds up to, starting from `seed`; failed, unreadable and read-only
/// calls are skipped.
pub(crate) fn fold(seed: Vec<TodoItem>, calls: &[CallRecord]) -> Vec<TodoItem> {
    let mut items = seed;
    for record in calls {
        if !record.ok {
            continue;
        }
        let Ok(args) = Args::fill(PARAMS, &record.args) else {
            continue;
        };
        let Ok(call) = Call::read(&args) else {
            continue;
        };
        if call.op.is_read_only() {
            continue;
        }
        let (next, errors) = ops::apply(&items, &call);
        if errors.is_empty() {
            items = next;
        }
    }
    items
}

/// The list a session holds: the newest `set` memo is the anchor, everything after it is folded in.
pub(crate) fn current(source: &dyn Source, session: i64) -> Result<Vec<TodoItem>> {
    let (seed, from) = match source.last_reset(session, RESET)? {
        Some((block, memo)) => (seed_of(&memo), block),
        None => (Vec::new(), 0),
    };
    let calls = source.calls_after(session, from, "todo")?;
    Ok(fold(seed, &calls))
}

/// The list a `set` result carried, read back from its memo.
fn seed_of(memo: &str) -> Vec<TodoItem> {
    serde_json::from_str::<serde_json::Value>(memo)
        .ok()
        .and_then(|value| value.get("items").cloned())
        .and_then(|items| serde_json::from_value(items).ok())
        .unwrap_or_default()
}
