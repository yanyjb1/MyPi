// The block tree: a full-screen navigator over the blocks of one conversation.

use std::collections::{BTreeMap, HashSet};

use serde_json::{Value, json};

use crate::ui::tui::zone::{
    Claim, Frame, Kind, Local, Memo, Outcome, Shared, Structure, TermSize, Zone, ZoneId,
};

use super::rows::Row;
use crate::ui::tui::action::{Action, Key, KeyCode, Mods};
use crate::ui::tui::lang::Lang;
use crate::ui::tui::theme::Accent;

// No modifiers held.
const PLAIN: Mods = Mods {
    ctrl: false,
    alt: false,
    shift: false,
};

// Shift held.
const SHIFT: Mods = Mods {
    ctrl: false,
    alt: false,
    shift: true,
};

// Whether Enter may rewind, or is only allowed to look.
const CHERISH_KEY: &str = "config.ui.tui.zone.tree.cherishYourWords";

// The display language the page is written in.
const LANG_KEY: &str = crate::ui::tui::lang::Lang::KEY;

// One block, as the tree draws it.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub kind: String,
    pub text: String,

    // The block's own text when it is a user message.
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

// The wire's block type, in the tree's shorter words.
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
    shared: Shared,
    size: Option<TermSize>,
    session: Option<i64>,
    structure: Structure,
    blocks: BTreeMap<i64, Node>,
    rows: Vec<Row>,
    // Indices into `rows` that passed the filter: filtering keeps indices, never reorders.
    visible: Vec<usize>,
    // Position inside `visible`.
    selected: usize,
    // First visible row (written back by the draw).
    scroll: usize,
    // The reader's filter words; empty keeps every row.
    filter: String,
    only_user: bool,
    // Shift+Enter rewinds instead of looking: `config.ui.tui.zone.tree.cherishYourWords`.
    cherish: bool,
    // The language the page is written in; `en` until the config answers.
    lang: Lang,
    // The cursor has found the newest block once; after that it stays where the reader put it.
    landed: bool,
    // The id ranges asked for and not yet answered.
    asking: HashSet<[i64; 2]>,
    // The rows are stale and must be rebuilt before the next draw.
    stale: bool,
    // Bumped whenever this zone's rows would differ.
    rev: u64,
}

impl Default for TreeZone {
    fn default() -> Self {
        Self {
            shared: Shared::default(),
            size: None,
            session: None,
            structure: Structure::default(),
            blocks: BTreeMap::new(),
            rows: Vec::new(),
            visible: Vec::new(),
            selected: 0,
            scroll: 0,
            filter: String::new(),
            only_user: false,
            cherish: false,
            lang: Lang::default(),
            landed: false,
            asking: HashSet::new(),
            stale: true,
            rev: 0,
        }
    }
}

impl TreeZone {
    // Rebuild the rows and the visible set, keeping the selected block where possible.
    fn sync(&mut self) {
        if !self.stale {
            return;
        }
        self.stale = false;
        let keep = self.row().map(|row| row.block_id);
        self.rows = super::source::rows(&self.structure, &self.blocks, self.only_user);
        self.refilter();
        let at = self
            .landed
            .then(|| keep.and_then(|id| self.position_of(id)))
            .flatten();
        if let Some(index) = at {
            self.selected = index;
        } else if self.rows.is_empty() {
            self.selected = 0;
        } else if self.landed {
            self.selected = self.selected.min(self.visible.len().saturating_sub(1));
        } else if let Some(index) = self.current() {
            // The newest block is only known once the blocks themselves are here.
            self.selected = index;
            self.landed = true;
        }
        self.scroll = self.scroll.min(self.visible.len().saturating_sub(1));
    }

    // Recompute the visible set. Only a row whose text is already here can match.
    fn refilter(&mut self) {
        let needle = self.filter.trim().to_lowercase();
        self.visible = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| {
                needle.is_empty() || super::render::haystack(row, self.lang).contains(&needle)
            })
            .map(|(i, _)| i)
            .collect();
        self.selected = self.selected.min(self.visible.len().saturating_sub(1));
    }

    // Where a block sits in the visible set.
    fn position_of(&self, id: i64) -> Option<usize> {
        self.visible
            .iter()
            .position(|slot| self.rows[*slot].block_id == id)
    }

    fn move_by(&mut self, delta: i64) {
        if self.visible.is_empty() {
            return;
        }
        let last = self.visible.len() - 1;
        let next = self.selected as i64 + delta;
        self.selected = next.clamp(0, last as i64) as usize;
    }

    // The rows the body may take, the header and the hints excluded.
    fn body(&self) -> usize {
        let size = self.size.unwrap_or(TermSize { cols: 80, rows: 24 });
        let hints = super::render::hints(self.lang, self.cherish, self.only_user);
        super::render::body_rows(size, &hints)
    }

    // Keep the window on the cursor: one screen of rows at most.
    pub(crate) fn settle_scroll(&mut self, body: usize) {
        if self.visible.is_empty() {
            self.scroll = 0;
            return;
        }
        let body = body.max(1);
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + body {
            self.scroll = self.selected + 1 - body;
        }
        self.scroll = self.scroll.min(self.visible.len().saturating_sub(body));
    }

    // Ask for the blocks the window shows and does not hold yet.
    fn wanted(&mut self) {
        if self.visible.is_empty() {
            return;
        }
        let body = self.body().max(1);
        self.settle_scroll(body);
        let from = self.scroll.min(self.visible.len());
        let to = (from + body).min(self.visible.len());
        let mut ids: Vec<i64> = self.visible[from..to]
            .iter()
            .map(|slot| self.rows[*slot].block_id)
            .filter(|id| !self.blocks.contains_key(id))
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

        let Some(session) = self.session else {
            return;
        };
        if fresh.is_empty() {
            return;
        }
        self.shared.mail.send(Frame::new(
            "call",
            "blocks",
            json!({"session": session, "ranges": fresh}),
        ));
    }

    // The structural range this block belongs to.
    fn span_of(&self, id: i64) -> Option<(i64, i64)> {
        self.structure
            .turns()
            .flat_map(|round| round.ranges.iter())
            .find(|[lo, hi]| *lo <= id && id <= *hi)
            .map(|[lo, hi]| (*lo, *hi))
    }

    // The newest visible row: rows are in block order, so it is the last one.
    fn current(&self) -> Option<usize> {
        (!self.visible.is_empty()).then(|| self.visible.len() - 1)
    }

    // The row under the cursor.
    fn row(&self) -> Option<&Row> {
        self.row_at(self.selected)
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
        if let Some(row) = self.row() {
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

    // The page is entered: the rows are rebuilt from scratch.
    fn enter(&mut self, size: TermSize, _memo: &Memo) {
        self.size = Some(size);
        self.stale = true;
        self.landed = false;
        self.asking.clear();
        self.sync();
        let Some(session) = self.session else {
            return;
        };
        for letter in [
            Frame::new("call", "init", json!({"session": session})),
            Frame::new("call", "config", json!({"key": CHERISH_KEY})),
            Frame::new("call", "config", json!({"key": LANG_KEY})),
        ] {
            self.shared.mail.send(letter);
        }
    }

    fn follow(&mut self, id: Option<i64>) {
        self.session = id;
        if id.is_none() {
            self.structure.clear();
            self.blocks.clear();
            self.rows.clear();
            self.visible.clear();
            self.filter.clear();
            self.selected = 0;
            self.scroll = 0;
            self.only_user = false;
            self.stale = true;
        }
    }

    fn read(&mut self, frame: &Frame) {
        match (frame.kind.as_str(), frame.name.as_str()) {
            ("call", "init") => {
                if let Some(snapshot) = frame.payload.get("structure") {
                    self.structure = Structure::of(snapshot);
                    self.asking.clear();
                    self.stale = true;
                }
                if let Some(rows) = frame.payload.get("blocks") {
                    self.merge(rows);
                }
                self.sync();
                self.wanted();
            }
            ("call", "blocks") => {
                if let Some(rows) = frame.payload.get("blocks") {
                    self.merge(rows);
                }
                self.sync();
                self.wanted();
            }
            ("call", "config") => match frame.payload.get("key").and_then(Value::as_str) {
                Some(CHERISH_KEY) => {
                    self.cherish = frame
                        .payload
                        .get("value")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                }
                Some(LANG_KEY) => {
                    if let Some(lang) = frame
                        .payload
                        .get("value")
                        .and_then(Value::as_str)
                        .and_then(Lang::parse)
                    {
                        self.lang = lang;
                    }
                }
                _ => {}
            },
            ("local", "resized") => {
                if let Some(size) = TermSize::of(&frame.payload) {
                    self.size = Some(size);
                }
            }
            _ => {}
        }
    }

    fn act(&mut self, action: &Action) -> (Outcome, Vec<Local>) {
        self.sync();
        match action {
            Action::ScrollUp { shift } => self.move_by(if *shift { -10 } else { -1 }),
            Action::ScrollDown { shift } => self.move_by(if *shift { 10 } else { 1 }),
            // A paste is a filter word here: a navigator is not worth multi-line semantics.
            Action::Paste(text) => {
                self.filter.push_str(&text.replace('\n', " "));
                self.scroll = 0;
                self.refilter();
            }
            Action::Key(key) => match key.code {
                KeyCode::Up => self.move_by(-1),
                KeyCode::Down => self.move_by(1),
                KeyCode::PageUp => self.move_by(-10),
                KeyCode::PageDown => self.move_by(10),
                KeyCode::Home => self.selected = 0,
                KeyCode::End => self.selected = self.visible.len().saturating_sub(1),
                KeyCode::Tab => {
                    self.only_user = !self.only_user;
                    self.stale = true;
                    self.scroll = 0;
                    self.sync();
                }
                KeyCode::Enter => {
                    let rewind = key.mods.shift == self.cherish;
                    if rewind {
                        if let Some(row) = self.row()
                            && let Some(session) = self.session
                        {
                            let content = json!({
                                "name": "rewind",
                                "content": {
                                    "session": session,
                                    "turn": row.turn,
                                    "block": row.block_id,
                                },
                            });
                            self.shared.mail.send(Frame::new("post", "meta", content));
                            return (self.handover(), Vec::new());
                        }
                    } else {
                        return (self.handover_focusing(), Vec::new());
                    }
                }
                KeyCode::Esc => return (self.handover(), Vec::new()),
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.scroll = 0;
                    self.refilter();
                }
                KeyCode::Char('u') if key.mods.ctrl => {
                    self.filter.clear();
                    self.scroll = 0;
                    self.refilter();
                }
                KeyCode::Char(c) if !key.mods.ctrl => {
                    self.filter.push(c);
                    self.scroll = 0;
                    self.refilter();
                }
                _ => {}
            },
        }
        self.wanted();
        (Outcome::Stay, Vec::new())
    }
}

impl Zone for TreeZone {
    fn bind(&mut self, shared: Shared) {
        self.shared = shared;
    }

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
            key(PLAIN, KeyCode::Backspace),
            Claim::AnyChar,
            Claim::Paste,
            Claim::Wheel,
            Claim::WheelShift,
        ]
    }

    fn kinds(&self) -> &'static [Kind] {
        &[Kind::Call]
    }

    fn start(&mut self, size: TermSize, memo: &Memo) -> Vec<Local> {
        self.enter(size, memo);
        self.bump();
        Vec::new()
    }

    fn session(&mut self, id: Option<i64>) -> Vec<Local> {
        self.follow(id);
        self.bump();
        Vec::new()
    }

    fn tick(&mut self) -> Vec<Local> {
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

    fn event(&mut self, frame: &Frame) -> Vec<Local> {
        self.read(frame);
        self.bump();
        Vec::new()
    }

    fn action(&mut self, action: &Action) -> (Outcome, Vec<Local>) {
        let out = self.act(action);
        self.bump();
        out
    }

    fn render(&mut self) -> Vec<ratatui::text::Line<'static>> {
        super::render::lines(self)
    }
}

impl TreeZone {
    pub(crate) fn lang(&self) -> Lang {
        self.lang
    }

    pub(crate) fn filter(&self) -> &str {
        &self.filter
    }

    pub(crate) fn only_user(&self) -> bool {
        self.only_user
    }

    pub(crate) fn cherish(&self) -> bool {
        self.cherish
    }

    pub(crate) fn turns(&self) -> usize {
        self.structure.turns().count()
    }

    pub(crate) fn visible_len(&self) -> usize {
        self.visible.len()
    }

    pub(crate) fn selected(&self) -> usize {
        self.selected
    }

    pub(crate) fn scroll(&self) -> usize {
        self.scroll
    }

    pub(crate) fn set_scroll(&mut self, top: usize) {
        self.scroll = top;
    }

    pub(crate) fn size(&self) -> Option<TermSize> {
        self.size
    }

    pub(crate) fn accent(&self) -> Accent {
        self.shared.palette.get()
    }

    // Row `i` of the visible set.
    pub(crate) fn row_at(&self, i: usize) -> Option<&Row> {
        self.rows.get(*self.visible.get(i)?)
    }
}
