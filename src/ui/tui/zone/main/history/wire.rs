//! One frame in: content to the local section, structure to its bookkeeping.

use serde_json::{Value, json};

use super::local::content::Block;
use super::local::section::Idx;
use super::local::slot::{Level, Life};
use super::product::Task;
use super::zone::HistoryZone;

use crate::ui::tui::TodoFold;
use crate::ui::tui::zone::{Frame, Request, Shout, Turns, name};

/// One frame, decided by its class and name.
pub(super) fn read_frame(zone: &mut HistoryZone, frame: &Frame) {
    match (frame.class.as_str(), frame.name.as_str()) {
        ("stream", _) => read_stream(zone, frame),
        ("call", "init") => {
            if let Some(rows) = frame.content.get("blocks") {
                take_blocks(zone, rows);
            }
            if let Some(todo) = frame.content.get("todo") {
                take_todo(zone, todo);
            }
            zone.outbox.push(Request::Structure);
        }
        ("call", "blocks") => {
            if let Some(rows) = frame.content.get("blocks") {
                take_blocks(zone, rows);
            }
            zone.asking
                .retain(|[lo, hi]| !(*lo..=*hi).all(|id| zone.local.has(id)));
        }
        ("call", "config") => read_config(zone, &frame.content),
        ("call", "todo") => {
            if let Some(todo) = frame.content.get("todo") {
                take_todo(zone, todo);
            }
        }
        _ => {}
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
        "change" => {
            if content.get("action").and_then(Value::as_str) == Some("rewind") {
                zone.outbox.push(Request::Structure);
            }
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

fn piece(zone: &mut HistoryZone, content: &Value) {
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
    match kind {
        "text" => zone.local.live_mut().text(text),
        "thinking" => zone.local.live_mut().thinking(text),
        "arguments" => zone.local.live_mut().arguments(call, text),
        "boundary" => zone.local.live_mut().boundary(),
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
            zone.local.live_mut().tool_start(call, name, args, intent);
        }
        "tool_progress" => zone.local.live_mut().tool_progress(call, text),
        "wake" => zone.local.live_mut().wake(text),
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
            if ok {
                moved(zone, memo);
            }
            zone.local.live_mut().tool_end(call, ok, result, memo, took);
        }
        _ => {}
    }
    zone.shouts
        .push(Shout::new(name::BUSY, json!({"on": true})));
    zone.shouts.push(Shout::new(name::TICK, json!({})));
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
    let handed = zone.local.handover(&ranges);
    if handed {
        let last = ranges.iter().map(|[_, hi]| *hi).max().unwrap_or(0);
        zone.landed = zone.landed.max(last);
    } else {
        zone.local.live_mut().clear();
        zone.run.clear();
    }
    for [lo, hi] in ranges {
        if (lo..=hi).all(|id| zone.local.has(id)) {
            continue;
        }
        zone.outbox.push(Request::Blocks {
            ranges: vec![[lo, hi]],
        });
    }
    zone.outbox.push(Request::Structure);
}

/// Decode a `blocks` array, keep the content, and number the fresh ones.
fn take_blocks(zone: &mut HistoryZone, rows: &Value) {
    let Some(items) = rows.as_array() else {
        return;
    };
    for row in items {
        let Some(block) = Block::of(row) else {
            continue;
        };
        let id = block.id;
        let fresh = id > zone.landed;
        let absent = !zone.local.has(id);
        let differs = zone.local.differs(&block);
        if absent || differs {
            zone.products.drop_block(id);
            if let Some(num) = zone.local.num_of(id) {
                zone.run.forget(Idx::Fixed(num));
            }
        }
        zone.local.put(block);
        if !fresh {
            continue;
        }
        zone.landed = id;
        if zone.needs_layout {
            continue;
        }
        zone.local.kill_short_lived();
        zone.local.append_id(id);
    }
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
                zone.view
                    .set_focus_line(u16::try_from(n).unwrap_or(u16::MAX));
            }
        }
        "config.ui.tui.zone.main.history.cardFoldBudget" => {
            if let Some(n) = number {
                zone.fold = (n as usize).max(3);
                zone.products.drop_fixed();
            }
        }
        "config.ui.tui.zone.main.history.cacheMargin" => {
            if let Some(n) = number {
                zone.products.set_margin((n as usize).max(1));
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
    zone.structure = snapshot.clone();
    if zone.needs_layout && !zone.structure.rounds().is_empty() {
        zone.needs_layout = false;
        lay_out(zone);
    } else {
        drop_unowned(zone);
    }
    zone.settle_focus();
    zone.ensure_around();
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
    for id in ids.iter().rev() {
        zone.local.append_older(*id);
    }
    zone.landed = ids.last().copied().unwrap_or(0);
    zone.run.clear();
    zone.view.follow();
}

/// Slots whose blocks the conversation no longer owns go dark, and their content with them.
fn drop_unowned(zone: &mut HistoryZone) {
    let gone = zone.local.unowned(|id| zone.structure.owns(id));
    for idx in &gone {
        zone.local.dead(*idx);
    }
    zone.local.retain(|id| zone.structure.owns(id));
    zone.run.clear();
    zone.landed = zone.local.max_id();
    if gone.is_empty() {
        return;
    }
    if let Some(text) = zone.local.last_user_text() {
        zone.shouts
            .push(Shout::new(name::PREFILL, json!({"text": text})));
    }
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
