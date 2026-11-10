//! Rebuild a session's list from its recorded `todo` calls. Pure: the same calls give the same list.

use anyhow::Result;

use super::ops::{self, Call};
use super::state::TodoItem;
use super::tool::PARAMS;
use crate::server::agent::tools::contract::view::CallRecord;
use crate::server::agent::tools::contract::view::Source;
use crate::server::agent::tools::spec::Args;

/// The list a `todo` result leaves in its memo; the newest such result is the replay's anchor.
const LIST: &str = "\"items\":";

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

/// The list a session holds: the newest list-carrying result is the anchor, and only the calls
/// after it are folded in.
pub(crate) fn current(source: &dyn Source, session: i64) -> Result<Vec<TodoItem>> {
    let anchor = source
        .last_memo(session, LIST)?
        .and_then(|(block, memo)| Some((block, seed_of(&memo)?)));
    let (seed, from) = match anchor {
        Some((block, items)) => (items, block),
        None => (Vec::new(), 0),
    };
    Ok(fold(seed, &source.calls_after(session, from, "todo")?))
}

/// The list a result memo carried, when it parses as one.
fn seed_of(memo: &str) -> Option<Vec<TodoItem>> {
    let value = serde_json::from_str::<serde_json::Value>(memo).ok()?;
    serde_json::from_value(value.get("items")?.clone()).ok()
}
