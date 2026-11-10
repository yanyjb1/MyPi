//! The block tree: a full-screen navigator over the blocks of one conversation.

use std::collections::{BTreeMap, HashSet};

use serde_json::Value;

use crate::ui::tui::zone::{
    Claim, Frame, Kind, Memo, Outcome, Request, TermSize, Turns, Zone, ZoneId,
};

use super::rows::Row;
use crate::ui::tui::action::{Action, Key, KeyCode, Mods};
use crate::ui::tui::theme::Accent;

/// No modifiers held.
const PLAIN: Mods = Mods {
    ctrl: false,
    alt: false,
    shift: false,
};

/// Shift held.
const SHIFT: Mods = Mods {
    ctrl: false,
    alt: false,
    shift: true,
};

/// Whether Enter may rewind, or is only allowed to look.
const CHERISH_KEY: &str = "config.ui.tui.zone.tree.cherishYourWords";

/// One block, as the tree draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub kind: String,
    pub text: String,
    /// The block's own text when it is a user message.
    pub body: Option<String>,
}

impl Node {
    fn of(row: &Value) -> Option<(i64, Node)> {
        let id = row.get("id")?.as_i64()?;
        let kind = short_kind(row.get("kind")?.as_str()?);
        let body = match kind {
            "user" => row
                .get("content")
                .or_else(|| row.get("text"))
                .and_then(Value::as_str)
                .map(str::to_string),
            _ => None,
        };
        let lead = ["content", "text", "name", "args", "result"]
            .iter()
            .filter_map(|key| row.get(*key).and_then(Value::as_str))
            .find(|text| !text.trim().is_empty())
            .unwrap_or("");
        let first = lead.lines().next().unwrap_or("").trim();
        let summary = if first.is_empty() {
            format!("{kind}: #{id}")
        } else {
            format!("{kind}: {first}")
        };
        Some((
            id,
            Node {
                kind: kind.to_string(),
                text: summary,
                body,
            },
        ))
    }
}

/// The wire's block type, in the tree's shorter words.
fn short_kind(kind: &str) -> &str {
    match kind {
        "user" => "user",
        "wake" => "wake",
        "assistant" => "assistant",
        "reasoning" => "reasoning",
        "tool_request" => "tool_call",
        "tool_result" => "tool_result",
        _ => "block",
    }
}

pub struct TreeZone {
    size: Option<TermSize>,
    structure: Turns,
    blocks: BTreeMap<i64, Node>,
    rows: Vec<Row>,
    selected: usize,
    scroll: usize,
    only_user: bool,
    /// Shift+Enter rewinds instead of looking: `config.ui.tui.zone.tree.cherishYourWords`.
    cherish: bool,
    /// The cursor has found the newest block once; after that it stays where the reader put it.
    landed: bool,
    /// The live accent, broadcast as a local frame.
    accent: Accent,
    /// The id ranges asked for and not yet answered.
    asking: HashSet<[i64; 2]>,
    /// The rows are stale and must be rebuilt before the next draw.
    stale: bool,
    /// Bumped whenever this zone's rows would differ.
    rev: u64,
}

impl Default for TreeZone {
    fn default() -> Self {
        Self {
            size: None,
            structure: Turns::default(),
            blocks: BTreeMap::new(),
            rows: Vec::new(),
            selected: 0,
            scroll: 0,
            only_user: false,
            cherish: false,
            landed: false,
            accent: Accent::default(),
            asking: HashSet::new(),
            stale: true,
            rev: 0,
        }
    }
}

impl TreeZone {
    /// Rebuild the rows, keeping the selected block where possible.
    fn sync(&mut self) {
        if !self.stale {
            return;
        }
        self.stale = false;
        let keep = self.rows.get(self.selected).map(|row| row.block_id);
        self.rows = super::source::rows(&self.structure, &self.blocks, self.only_user);
        let at = self
            .landed
            .then(|| keep.and_then(|id| self.rows.iter().position(|row| row.block_id == id)))
            .flatten();
        if let Some(index) = at {
            self.selected = index;
        } else if self.rows.is_empty() {
            self.selected = 0;
        } else if self.landed {
            self.selected = self.selected.min(self.rows.len() - 1);
        } else if let Some(index) = self.current() {
            // The newest block is only known once the blocks themselves are here.
            self.selected = index;
            self.landed = true;
        }
        self.scroll = self.scroll.min(self.rows.len().saturating_sub(1));
    }

    fn move_by(&mut self, delta: i64) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() - 1;
        let next = self.selected as i64 + delta;
        self.selected = next.clamp(0, last as i64) as usize;
    }

    /// The rows the screen has, the hint line excluded.
    fn body(&self) -> usize {
        let height = self.size.map(|size| size.rows as usize).unwrap_or(24);
        height.saturating_sub(1)
    }

    /// Keep the window on the cursor: one screen of rows at most.
    fn settle_scroll(&mut self, body: usize) {
        if self.rows.is_empty() {
            self.scroll = 0;
            return;
        }
        let body = body.max(1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + body {
            self.scroll = self.selected + 1 - body;
        }
        self.scroll = self.scroll.min(self.rows.len().saturating_sub(body));
    }

    /// Ask for the blocks the window shows and does not hold yet.
    fn wanted(&mut self) -> Vec<Request> {
        if self.rows.is_empty() {
            return Vec::new();
        }
        let body = self.body().max(1);
        self.settle_scroll(body);
        let from = self.scroll.min(self.rows.len());
        let to = (from + body).min(self.rows.len());
        let mut ids: Vec<i64> = self.rows[from..to]
            .iter()
            .filter(|row| !self.blocks.contains_key(&row.block_id))
            .map(|row| row.block_id)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        // The server answers one structural range at a time: a run never crosses one.
        let mut ranges: Vec<[i64; 2]> = Vec::new();
        for id in ids {
            let Some((start, _)) = self.span_of(id) else {
                continue;
            };
            match ranges.last_mut() {
                Some(last) if last[1] + 1 == id && last[0] >= start => last[1] = id,
                _ => ranges.push([id, id]),
            }
        }
        let fresh: Vec<[i64; 2]> = ranges
            .into_iter()
            .filter(|range| self.asking.insert(*range))
            .collect();
        if fresh.is_empty() {
            Vec::new()
        } else {
            vec![Request::Blocks { ranges: fresh }]
        }
    }

    /// The structural range this block belongs to.
    fn span_of(&self, id: i64) -> Option<(i64, i64)> {
        self.structure
            .rounds()
            .iter()
            .flat_map(|round| round.ranges.iter())
            .find(|[lo, hi]| *lo <= id && id <= *hi)
            .map(|[lo, hi]| (*lo, *hi))
    }

    /// The newest row: rows are in block order, so it is the last one.
    fn current(&self) -> Option<usize> {
        (!self.rows.is_empty()).then(|| self.rows.len() - 1)
    }

    fn merge(&mut self, rows: &Value) {
        for row in rows.as_array().into_iter().flatten() {
            if let Some((id, node)) = Node::of(row) {
                self.blocks.insert(id, node);
            }
        }
        self.stale = true;
    }

    fn handover(&mut self) -> Outcome {
        Outcome::Show {
            to: ZoneId::Main,
            memo: Vec::new(),
        }
    }

    fn handover_focusing(&mut self) -> Outcome {
        let mut memo = Vec::new();
        if let Some(row) = self.rows.get(self.selected) {
            memo.push(("tree.focus".to_string(), row.block_id.to_string()));
        }
        Outcome::Show {
            to: ZoneId::Main,
            memo,
        }
    }

    fn bump(&mut self) {
        self.rev = self.rev.wrapping_add(1);
    }

    /// The page is entered: the rows are rebuilt from scratch.
    fn enter(&mut self, size: TermSize, _memo: &Memo) -> Vec<Request> {
        self.size = Some(size);
        self.stale = true;
        self.landed = false;
        self.asking.clear();
        self.sync();
        vec![
            Request::Structure,
            Request::Init,
            Request::Config {
                key: CHERISH_KEY.to_string(),
            },
        ]
    }

    fn follow(&mut self, id: Option<i64>) -> Vec<Request> {
        if id.is_none() {
            self.structure.clear();
            self.blocks.clear();
            self.rows.clear();
            self.selected = 0;
            self.scroll = 0;
            self.only_user = false;
            self.stale = true;
        }
        Vec::new()
    }

    fn read(&mut self, frame: &Frame) -> Vec<Request> {
        match (frame.class.as_str(), frame.name.as_str()) {
            ("call", "init") | ("call", "blocks") => {
                if let Some(rows) = frame.content.get("blocks") {
                    self.merge(rows);
                }
                self.sync();
                return self.wanted();
            }
            ("call", "config") => {
                if frame.content.get("key").and_then(Value::as_str) == Some(CHERISH_KEY) {
                    self.cherish = frame
                        .content
                        .get("value")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                }
            }
            ("local", "resized") => {
                if let Some(size) = TermSize::of(&frame.content) {
                    self.size = Some(size);
                }
            }
            ("local", "accent") => {
                if let Some(accent) = frame
                    .content
                    .get("color")
                    .and_then(Value::as_str)
                    .and_then(Accent::parse)
                {
                    self.accent = accent;
                }
            }
            _ => {}
        }
        Vec::new()
    }

    fn act(&mut self, action: &Action) -> (Outcome, Vec<Request>) {
        self.sync();
        match action {
            Action::ScrollUp { shift } => self.move_by(if *shift { -10 } else { -1 }),
            Action::ScrollDown { shift } => self.move_by(if *shift { 10 } else { 1 }),
            Action::Paste(_) => {}
            Action::Key(key) => match key.code {
                KeyCode::Up => self.move_by(-1),
                KeyCode::Down => self.move_by(1),
                KeyCode::PageUp => self.move_by(-10),
                KeyCode::PageDown => self.move_by(10),
                KeyCode::Home => self.selected = 0,
                KeyCode::End => self.selected = self.rows.len().saturating_sub(1),
                KeyCode::Tab => {
                    self.only_user = !self.only_user;
                    self.stale = true;
                    self.sync();
                }
                KeyCode::Enter => {
                    let rewind = key.mods.shift == self.cherish;
                    if rewind {
                        if let Some(row) = self.rows.get(self.selected) {
                            let request = Request::Rewind {
                                turn: row.turn,
                                block: row.block_id,
                            };
                            return (self.handover(), vec![request]);
                        }
                    } else {
                        return (self.handover_focusing(), Vec::new());
                    }
                }
                KeyCode::Esc | KeyCode::Char('q') => return (self.handover(), Vec::new()),
                _ => {}
            },
        }
        (Outcome::Stay, self.wanted())
    }

    fn take_structure(&mut self, snapshot: &Turns) -> Vec<Request> {
        self.structure = snapshot.clone();
        self.stale = true;
        self.asking.clear();
        self.sync();
        self.wanted()
    }
}

impl Zone for TreeZone {
    fn keys(&self) -> Vec<Claim> {
        let key = |mods, code| Claim::Key(Key { code, mods });
        vec![
            key(PLAIN, KeyCode::Up),
            key(PLAIN, KeyCode::Down),
            key(PLAIN, KeyCode::PageUp),
            key(PLAIN, KeyCode::PageDown),
            key(PLAIN, KeyCode::Home),
            key(PLAIN, KeyCode::End),
            key(PLAIN, KeyCode::Enter),
            key(SHIFT, KeyCode::Enter),
            key(PLAIN, KeyCode::Tab),
            key(PLAIN, KeyCode::Esc),
            key(PLAIN, KeyCode::Char('q')),
            Claim::Wheel,
            Claim::WheelShift,
        ]
    }

    fn kinds(&self) -> &'static [Kind] {
        &[Kind::Call]
    }

    fn start(&mut self, size: TermSize, memo: &Memo) -> Vec<Request> {
        let out = self.enter(size, memo);
        self.bump();
        out
    }

    fn session(&mut self, id: Option<i64>) -> Vec<Request> {
        let out = self.follow(id);
        self.bump();
        out
    }

    fn tick(&mut self) -> Vec<Request> {
        let before = (self.selected, self.scroll);
        self.sync();
        let body = self.body();
        self.settle_scroll(body);
        if (self.selected, self.scroll) != before {
            self.bump();
        }
        Vec::new()
    }

    fn revision(&self) -> u64 {
        self.rev
    }

    fn event(&mut self, frame: &Frame) -> Vec<Request> {
        let out = self.read(frame);
        self.bump();
        out
    }

    fn action(&mut self, action: &Action) -> (Outcome, Vec<Request>) {
        let out = self.act(action);
        self.bump();
        out
    }

    fn structure(&mut self, snapshot: &Turns) -> Vec<Request> {
        let out = self.take_structure(snapshot);
        self.bump();
        out
    }

    fn render(&mut self) -> Vec<ratatui::text::Line<'static>> {
        super::render::rows(
            &self.rows,
            self.selected,
            self.size,
            &mut self.scroll,
            &self.accent,
        )
    }
}
