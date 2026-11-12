//! One frame in: content to the local section, structure to its bookkeeping.

use serde_json::{Value, json};

use super::focus::Target;
use super::local::content::{Block, Kind as BlockKind, Truncation};
use super::local::mutable::Entry;
use super::local::section::Idx;
use super::local::slot::{Level, Life};
use super::product::Task;
use super::zone::HistoryZone;

use crate::ui::tui::TodoFold;
use crate::ui::tui::zone::{Frame, Request, Round, Shout, Turns, name};

/// One frame, decided by its class and name.
pub(super) fn read_frame(zone: &mut HistoryZone, frame: &Frame) {
    match (frame.class.as_str(), frame.name.as_str()) {
        ("stream", _) => read_stream(zone, frame),
        ("call", "init") => {
            // The wire orders rounds oldest first; a position is laid newest first.
            if let Some(snapshot) = frame.content.get("structure").and_then(turns_of) {
                apply_structure(zone, &snapshot);
            }
            if let Some(rows) = frame.content.get("blocks") {
                take_blocks(zone, rows);
            }
            if let Some(todo) = frame.content.get("todo") {
                take_todo(zone, todo);
            }
        }
        ("call", "blocks") => {
            if let Some(rows) = frame.content.get("blocks") {
                take_blocks(zone, rows);
            }
        }
        ("call", "config") => read_config(zone, &frame.content),
        ("local", "opened") => read_opened(zone, &frame.content),
        _ => {}
    }
}

/// The server's opening word: what the configuration had to say at load, as transcript lines.
fn read_opened(zone: &mut HistoryZone, content: &Value) {
    let Some(notices) = content.get("notices").and_then(Value::as_array) else {
        return;
    };
    for notice in notices {
        let Some(text) = notice.get("text").and_then(Value::as_str) else {
            continue;
        };
        let level = match notice.get("level").and_then(Value::as_str) {
            Some("error") => Level::Error,
            Some("warn") => Level::Warn,
            _ => Level::Info,
        };
        zone.notice(text.to_string(), level, Life::Long);
    }
}

fn read_stream(zone: &mut HistoryZone, frame: &Frame) {
    let content = &frame.content;
    match frame.name.as_str() {
        "notice" => {
            let level = match content.get("level").and_then(Value::as_str) {
                Some("error") => Level::Error,
                Some("warn") => Level::Warn,
                _ => Level::Info,
            };
            let text = content
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let life = if level == Level::Error {
                Life::Long
            } else {
                Life::Short
            };
            zone.notice(text, level, life);
        }
        "signal" => {
            if content.get("signal").and_then(Value::as_str) == Some("rewind") {
                zone.pin.clear();
                zone.tick = 0;
                // The cut rewrote the rounds: whatever a round was waiting on names rows the
                // session no longer has, and nothing may stay exempt from the budget for them.
                zone.pending = None;
                zone.asking.clear();
                zone.outbox.push(Request::Structure);
            }
            zone.shouts.push(Shout::new(name::SIGNAL, content.clone()));
        }
        _ => {
            if content.get("ranges").is_some() {
                closed(zone, content);
                return;
            }
            piece(zone, content);
        }
    }
}

/// One piece: the model grows by an append, and the position it touched is marked.
fn piece(zone: &mut HistoryZone, content: &Value) {
    zone.tick += 1;
    if !zone.busy {
        zone.busy = true;
        zone.shouts
            .push(Shout::new(name::BUSY, json!({"on": true})));
    }
    let kind = content
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let text = content
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let call = content
        .get("call_id")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if let Some(usage) = content.get("usage").filter(|value| !value.is_null()) {
        let prompt = usage.get("prompt").and_then(Value::as_u64).unwrap_or(0);
        if prompt > 0 {
            zone.shouts.push(Shout::new(name::USAGE, json!({})));
        }
    }
    let touched = match kind {
        // A text piece grows its block in place: the product keeps its closed segments.
        "text" => {
            zone.thinking_ended();
            zone.local.running_mut().text(text);
            None
        }
        "thinking" => {
            zone.thinking_started();
            zone.local.running_mut().thinking(text);
            None
        }
        "arguments" => {
            let name = content
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            Some(zone.local.running_mut().arguments(call, name, text))
        }
        "boundary" => {
            zone.thinking_ended();
            zone.local.running_mut().boundary();
            None
        }
        "tool_start" => {
            let name = content
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let args = content
                .get("args")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let intent = content.get("intent").and_then(Value::as_str);
            zone.call_started(call, intent);
            Some(
                zone.local
                    .running_mut()
                    .tool_start(call, name, args, intent),
            )
        }
        "tool_progress" => Some(zone.local.running_mut().tool_progress(call, text)),
        "wake" => Some(zone.local.running_mut().wake(text)),
        "tool_end" => {
            let ok = content.get("ok").and_then(Value::as_bool).unwrap_or(false);
            let result = content
                .get("result")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let memo = content.get("memo").and_then(Value::as_str);
            let took = content
                .get("duration_ms")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let truncated = Truncation::of(content.get("truncated"));
            zone.call_ended(call);
            if ok {
                moved(zone, memo);
            }
            if let Some(memo) = memo {
                take_todo_memo(zone, call, memo);
            }
            Some(
                zone.local
                    .running_mut()
                    .tool_end(call, ok, result, memo, took, truncated),
            )
        }
        _ => None,
    };
    if let Some(at) = touched {
        zone.mark(Idx::Turn(at as u32));
    }
    for (at, from) in zone.local.running_mut().take_grown() {
        zone.grow(Idx::Turn(at as u32), from);
    }
    zone.shouts.push(Shout::new(name::TICK, json!({})));
}

/// A todo result answers with the whole list: the panel takes it.
fn take_todo_memo(zone: &mut HistoryZone, call: &str, memo: &str) {
    let name = zone
        .local
        .running()
        .request_at(call)
        .and_then(|at| zone.local.running().block(at))
        .and_then(|block| block.name.clone());
    if name.as_deref() != Some("todo") {
        return;
    }
    let Some(items) = serde_json::from_str::<Value>(memo)
        .ok()
        .and_then(|value| value.get("items").cloned())
    else {
        return;
    };
    take_todo(zone, &items);
}

/// A tool that moved the session: the cards after it relativize from the new directory.
fn moved(zone: &mut HistoryZone, memo: Option<&str>) {
    let Some(memo) = memo.and_then(|memo| serde_json::from_str::<Value>(memo).ok()) else {
        return;
    };
    if memo.get("kind").and_then(Value::as_str) != Some("cwd") {
        return;
    }
    let Some(path) = memo.get("path").and_then(Value::as_str) else {
        return;
    };
    zone.cwd.clear();
    zone.cwd.push_str(path);
    zone.shouts.push(Shout::new(
        name::CWD,
        json!({"path": path, "temporary": true}),
    ));
}

/// A round closed: the running turn's blocks take their numbers.
fn closed(zone: &mut HistoryZone, content: &Value) {
    let ranges = content
        .get("ranges")
        .and_then(ranges_of)
        .unwrap_or_default();
    if ranges.is_empty() {
        zone.local.drop_turn();
        zone.run.clear();
        zone.outbox.push(Request::Structure);
    } else {
        // A call that did not finish cleanly states why in the row the turn kept, not in the stream.
        let failed = has_failure(zone);
        if zone.local.handover(&ranges) {
            zone.run.clear();
            if failed {
                ask_blocks(zone, &ranges);
            }
            zone.outbox.push(Request::Structure);
        } else {
            // The running turn does not line up with the receipt: the receipt is the truth, so
            // its rows are fetched and handed over once they are all in hand.
            zone.pending = Some(ranges.clone());
            zone.local.reserve(&ranges);
            ask_blocks(zone, &ranges);
            settle_pending(zone);
        }
    }
    zone.pin.clear();
    // The receipt is the fact: the round is over, so the idle call goes out whether or not this
    // zone ever saw a piece — a front end that opened the round never told this zone it was busy.
    if zone.pending.is_none() {
        zone.busy = false;
        zone.shouts
            .push(Shout::new(name::BUSY, json!({"on": false})));
    }
    // The running region lives exactly as long as one round: once the receipt has landed and
    // nothing is owed by the backend, it must be empty.
    debug_assert!(
        zone.pending.is_some() || zone.local.running().entries().is_empty(),
        "the running region outlived its round"
    );
}

/// Whether this turn holds a call that failed: its reason lives in the stored row alone.
fn has_failure(zone: &HistoryZone) -> bool {
    zone.local
        .running()
        .entries()
        .iter()
        .any(|entry| match entry {
            Entry::Block(block) => block.kind == BlockKind::ToolResult && block.ok == Some(false),
            Entry::Notice { .. } => false,
        })
}

/// Ask for a receipt's rows. One round is one request: its rows arrive together, so the
/// handover it feeds is one event. Every range asked for is noted so the draw does not ask again.
fn ask_blocks(zone: &mut HistoryZone, ranges: &[[i64; 2]]) {
    if ranges.is_empty() {
        return;
    }
    for range in ranges {
        zone.asking.insert(*range);
    }
    zone.outbox.push(Request::Blocks {
        ranges: ranges.to_vec(),
    });
}

/// Finish a waiting close once the receipt's rows are all in hand.
///
/// `asking` is the one record of what is still owed: `take_blocks` drops a range from it the
/// moment every id of that range has its content, so the round is done when none of its ranges
/// is left there.
fn settle_pending(zone: &mut HistoryZone) {
    let ready = zone
        .pending
        .as_ref()
        .is_some_and(|ranges| ranges.iter().all(|range| !zone.asking.contains(range)));
    if !ready {
        return;
    }
    zone.pending = None;
    zone.local.drop_turn();
    zone.run.clear();
    zone.busy = false;
    zone.shouts
        .push(Shout::new(name::BUSY, json!({"on": false})));
    zone.outbox.push(Request::Structure);
}

/// Decode a `blocks` array: every row carries its id and takes the slot that id owns.
fn take_blocks(zone: &mut HistoryZone, rows: &Value) {
    let Some(items) = rows.as_array() else {
        return;
    };
    for row in items {
        let Some(block) = Block::of(row) else {
            continue;
        };
        let id = block.id;
        let absent = !zone.local.has(id);
        let differs = zone.local.differs(&block);
        zone.local.put(block);
        if absent || differs {
            for side in zone.local.exchange(id).into_iter().flatten() {
                if let Some(num) = zone.local.num_of(side) {
                    zone.local.drop_product(Idx::Fixed(num));
                    zone.forget(num);
                }
            }
        }
    }
    // A range whose blocks are all in hand is no longer waiting: it stops exempting content.
    // This runs before the wait is judged, because `asking` is what judges it.
    zone.asking
        .retain(|[lo, hi]| !(*lo..=*hi).all(|id| zone.local.has(id)));
    settle_pending(zone);
    zone.settle_focus();
}

/// The checklist, as one flat list.
fn take_todo(zone: &mut HistoryZone, value: &Value) {
    if value.is_null() {
        zone.todo.clear();
        zone.todo_panel.changed();
        return;
    }
    let Some(items) = value.as_array() else {
        return;
    };
    zone.todo = items
        .iter()
        .filter_map(|item| {
            Some(Task {
                text: item.get("text")?.as_str()?.to_string(),
                done: item.get("done").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect();
    zone.todo_panel.changed();
}

/// One config answer: the keys this zone reads.
fn read_config(zone: &mut HistoryZone, content: &Value) {
    let Some(key) = content.get("key").and_then(Value::as_str) else {
        return;
    };
    let number = content.get("value").and_then(Value::as_u64);
    match key {
        "config.ui.tui.zone.main.history.focusLine" => {
            if let Some(n) = number {
                zone.focus_line = usize::try_from(n).unwrap_or(usize::MAX);
            }
        }
        "config.ui.tui.zone.main.history.cardFoldBudget" => {
            if let Some(n) = number {
                zone.fold = (n as usize).max(3);
                zone.local.drop_products();
                zone.run.clear();
            }
        }
        "config.ui.tui.zone.main.history.cacheMargin" => {
            if let Some(n) = number {
                zone.margin = (n as usize).max(1);
            }
        }
        "config.ui.tui.zone.main.history.scrollHeight" => {
            if let Some(n) = number {
                zone.scroll_height = (n as usize).max(1);
            }
        }
        "config.ui.tui.zone.main.history.todoFold" => {
            if let Some(value) = content.get("value") {
                zone.todo_fold = match value {
                    Value::Bool(on) => TodoFold::On(*on),
                    Value::Number(n) => TodoFold::Capped(n.as_u64().unwrap_or(0) as usize),
                    _ => zone.todo_fold,
                };
                zone.todo_panel.changed();
            }
        }
        _ => {}
    }
}

/// The structural window arrived.
pub(super) fn apply_structure(zone: &mut HistoryZone, snapshot: &Turns) {
    zone.settle_dirty();
    zone.structure = snapshot.clone();
    let empty = zone.structure.rounds().is_empty();
    if zone.needs_layout && !empty {
        zone.needs_layout = false;
        // A section that already holds this session's own blocks was laid out by its receipts.
        if !zone.local.has_now() {
            lay_out(zone);
        }
    } else if !zone.needs_layout {
        drop_unowned(zone);
    }
    drop_stale_focus(zone);
    zone.settle_focus();
    zone.ensure_around();
}

/// A snapshot is authoritative: a block it does not own cannot be focused.
fn drop_stale_focus(zone: &mut HistoryZone) {
    let stale = match zone.pending_focus {
        Some(Target::Block { id, .. }) => !zone.structure.owns(id),
        Some(Target::Round(turn)) => !zone.structure.rounds().iter().any(|round| round.id == turn),
        Some(Target::Position { idx, .. }) => match idx {
            Idx::Fixed(_) => zone.local.slot(idx).is_none(),
            Idx::Turn(at) => at as usize >= zone.local.running().len(),
        },
        Some(Target::Running) | None => false,
    };
    if stale {
        zone.pending_focus = None;
    }
}

/// Lay the whole structure out as the past, newest block at -1.
fn lay_out(zone: &mut HistoryZone) {
    let turns = zone.structure.rounds();
    if turns.is_empty() {
        return;
    }
    let mut ids: Vec<i64> = Vec::new();
    for turn in turns.iter().rev() {
        for [lo, hi] in &turn.ranges {
            ids.extend(*lo..=*hi);
        }
    }
    zone.local.clear();
    zone.run.clear();
    for id in ids.iter().rev() {
        zone.local.append_older(*id);
    }
    zone.range.follow();
}

/// Slots whose blocks the conversation no longer owns go dark, and their content with them.
fn drop_unowned(zone: &mut HistoryZone) {
    let gone = zone.local.unowned(|id| zone.structure.owns(id));
    for idx in &gone {
        zone.local.dead(*idx);
    }
    if gone.is_empty() {
        return;
    }
    zone.run.clear();
    zone.local.drop_products();
    if let Some(text) = zone.local.last_user_text() {
        zone.shouts
            .push(Shout::new(name::PREFILL, json!({"text": text})));
    }
}

/// An `init` answer's `structure`: one `{turn, ranges}` per round, oldest first on the wire.
fn turns_of(value: &Value) -> Option<Turns> {
    let mut rounds = Vec::new();
    for item in value.as_array()? {
        let id = item.get("turn").and_then(Value::as_i64)?;
        let ranges = item.get("ranges").and_then(ranges_of)?;
        rounds.push(Round::new(id, ranges));
    }
    rounds.reverse();
    Some(Turns::of(rounds))
}

/// A frame's ranges: `[[lo, hi], …]`.
fn ranges_of(value: &Value) -> Option<Vec<[i64; 2]>> {
    let mut out = Vec::new();
    for item in value.as_array()? {
        let pair = item.as_array()?;
        if pair.len() != 2 {
            return None;
        }
        out.push([pair[0].as_i64()?, pair[1].as_i64()?]);
    }
    Some(out)
}
