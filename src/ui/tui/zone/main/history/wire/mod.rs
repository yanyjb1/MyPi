//! Frames in: pieces, closed rounds, notices, config answers, structure.

pub(crate) mod ask;
pub(crate) mod decode;
pub(crate) mod live;
mod transfer;

use serde_json::{Value, json};

use self::decode::{Block, Kind as BlockKind};
use super::local::{Level, Life, SlotKind};
use super::paint::todo::Task;
use super::zone::HistoryZone;

use crate::ui::tui::TodoFold;
use crate::ui::tui::zone::{Frame, Request, Shout, Turns, name};

impl HistoryZone {
    pub(super) fn read_frame(&mut self, frame: &Frame) {
        match (frame.class.as_str(), frame.name.as_str()) {
            ("stream", _) => self.read_stream(frame),
            ("call", "init") => {
                if let Some(rows) = frame.content.get("blocks") {
                    self.take_blocks(rows);
                }
                if let Some(todo) = frame.content.get("todo") {
                    self.take_todo(todo);
                }
                self.outbox.push(Request::Structure);
            }
            ("call", "blocks") => {
                if let Some(rows) = frame.content.get("blocks") {
                    self.take_blocks(rows);
                }
                self.asking
                    .retain(|[lo, hi]| !(*lo..=*hi).all(|id| self.blocks.contains_key(&id)));
            }
            ("call", "config") => self.read_config(&frame.content),
            ("call", "todo") => {
                if let Some(todo) = frame.content.get("todo") {
                    self.take_todo(todo);
                }
            }
            _ => {}
        }
    }

    /// One frame's content: a live piece, a closed round, a notice, a change.
    fn read_stream(&mut self, frame: &Frame) {
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
                self.notice(text, level, life);
            }
            "change" => {
                if content.get("action").and_then(Value::as_str) == Some("rewind") {
                    self.outbox.push(Request::Structure);
                }
            }
            _ => {
                if content.get("ranges").is_some() {
                    self.closed(content);
                    return;
                }
                self.piece(content);
            }
        }
    }

    /// One live piece from the running turn.
    fn piece(&mut self, content: &Value) {
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
        if let Some(usage) = content.get("usage").filter(|v| !v.is_null()) {
            let prompt = usage.get("prompt").and_then(Value::as_u64).unwrap_or(0);
            let completion = usage.get("completion").and_then(Value::as_u64).unwrap_or(0);
            let used = prompt + completion;
            if prompt > 0 && used != self.usage {
                self.usage = used;
                self.shouts
                    .push(Shout::new(name::USAGE, json!({"used": used})));
            }
        }
        match kind {
            "text" => self.live.text(text),
            "thinking" => self.live.thinking(text),
            "arguments" => self.live.arguments(call, text),
            "boundary" => self.live.boundary(),
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
                self.live.tool_start(call, name, args, intent);
            }
            "tool_progress" => self.live.tool_progress(call, text),
            "wake" => self.live.wake(text),
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
                self.live.tool_end(call, ok, result, memo, took);
            }
            _ => {}
        }
        self.shouts
            .push(Shout::new(name::BUSY, json!({"on": true})));
    }

    /// A round closed: the tail hands its product over and stops growing.
    fn closed(&mut self, content: &Value) {
        self.shouts
            .push(Shout::new(name::BUSY, json!({"on": false})));
        let ranges = content
            .get("ranges")
            .and_then(ranges_of)
            .unwrap_or_default();
        let handed = self.handover(&ranges);
        if !handed {
            self.live.clear();
            self.prep.rows.clear();
        }
        for [lo, hi] in ranges {
            let have = (lo..=hi).all(|id| self.blocks.contains_key(&id));
            if have {
                continue;
            }
            self.outbox
                .push(Request::call("blocks", json!({"ranges": [[lo, hi]]})));
        }
        self.outbox.push(Request::Structure);
    }

    /// Decode a `blocks` array, keep the rows, and give the newest ones their numbers.
    fn take_blocks(&mut self, rows: &Value) {
        let Some(items) = rows.as_array() else {
            return;
        };
        for row in items {
            let Some(block) = Block::of(row) else {
                continue;
            };
            let id = block.id;
            let used = block.usage.context();
            if block.usage.prompt > 0 && used != self.usage {
                self.usage = used;
                self.shouts
                    .push(Shout::new(name::USAGE, json!({"used": used})));
            }
            let fresh = id > self.landed;
            let absent = !self.blocks.contains_key(&id);
            let moved = self
                .blocks
                .get(&id)
                .is_some_and(|old| old.kind != block.kind || old.text != block.text);
            if absent || moved {
                self.products.drop_block(id);
                if let Some(num) = self.local.num_of(id) {
                    self.forget_num(num);
                }
            }
            self.blocks.insert(id, block);
            if !fresh {
                continue;
            }
            self.landed = id;
            if self.needs_layout {
                continue;
            }
            self.local.kill_short_lived();
            self.local.push_now(SlotKind::Block(id));
            self.products.drop_block(id);
        }
    }

    /// The checklist, as one flat list.
    fn take_todo(&mut self, value: &Value) {
        if value.is_null() {
            self.todo.clear();
            self.todo_panel.changed();
            return;
        }
        let Some(items) = value.as_array() else {
            return;
        };
        self.todo = items
            .iter()
            .filter_map(|item| {
                Some(Task {
                    text: item.get("text")?.as_str()?.to_string(),
                    done: item.get("done").and_then(Value::as_bool).unwrap_or(false),
                })
            })
            .collect();
        self.todo_panel.changed();
    }

    /// One config answer: the keys this zone reads.
    fn read_config(&mut self, content: &Value) {
        let Some(key) = content.get("key").and_then(Value::as_str) else {
            return;
        };
        let number = content.get("value").and_then(Value::as_u64);
        match key {
            "config.ui.tui.zone.main.history.focusLine" => {
                if let Some(n) = number {
                    self.disp.set_line(u16::try_from(n).unwrap_or(u16::MAX));
                }
            }
            "config.ui.tui.zone.main.history.cardFoldBudget" => {
                if let Some(n) = number {
                    self.fold = (n as usize).max(3);
                    self.products.drop_fixed();
                }
            }
            "config.ui.tui.zone.main.history.cacheMargin" => {
                if let Some(n) = number {
                    self.products.set_margin((n as usize).max(1));
                }
            }
            "config.ui.tui.zone.main.history.scrollHeight" => {
                if let Some(n) = number {
                    self.scroll_height = (n as usize).max(1);
                }
            }
            "config.ui.tui.zone.main.history.todoFold" => {
                if let Some(value) = content.get("value") {
                    self.todo_fold = match value {
                        Value::Bool(on) => TodoFold::On(*on),
                        Value::Number(n) => TodoFold::Capped(n.as_u64().unwrap_or(0) as usize),
                        _ => self.todo_fold,
                    };
                    self.todo_panel.changed();
                }
            }
            _ => {}
        }
    }

    pub(super) fn apply_structure(&mut self, snapshot: &Turns) {
        self.structure = snapshot.clone();
        if self.needs_layout && !self.structure.rounds().is_empty() {
            self.needs_layout = false;
            self.lay_out();
        } else {
            self.drop_unowned();
        }
        if let Some((_, block)) = self.pending_focus
            && let Some(num) = self.local.num_of(block)
        {
            self.pending_focus = None;
            self.focus_num(num);
        }
        self.ensure_around();
    }

    /// Lay the whole structure out as the past, newest block at -1.
    fn lay_out(&mut self) {
        let turns = self.structure.rounds();
        if turns.is_empty() {
            return;
        }
        let mut ids: Vec<i64> = Vec::new();
        for turn in turns.iter().rev() {
            for [lo, hi] in &turn.ranges {
                ids.extend(*lo..=*hi);
            }
        }
        self.local.clear();
        for id in ids.iter().rev() {
            self.local.push_older(SlotKind::Block(*id));
        }
        self.landed = ids.last().copied().unwrap_or(0);
        self.prep.rows.clear();
        self.disp.follow();
    }

    /// Slots whose blocks the session no longer owns go, and the tail with them.
    fn drop_unowned(&mut self) {
        let keep: Vec<i64> = self
            .local
            .iter()
            .filter_map(|slot| match slot.kind {
                SlotKind::Block(id) if !self.structure.owns(id) => Some(slot.num),
                _ => None,
            })
            .collect();
        let cut = keep
            .first()
            .copied()
            .unwrap_or(self.local.newest().unwrap_or(0) + 1);
        if cut >= 0 {
            self.local.truncate_now(cut as usize);
        }
        self.blocks.retain(|id, _| self.structure.owns(*id));
        self.prep.rows.clear();
        self.landed = self.blocks.keys().next_back().copied().unwrap_or(0);
        let retyped = self
            .blocks
            .values()
            .next_back()
            .filter(|block| block.kind == BlockKind::User)
            .and_then(|block| block.content.clone().or_else(|| block.text.clone()));
        if let Some(text) = retyped {
            self.shouts
                .push(Shout::new(name::PREFILL, json!({"text": text})));
        }
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
