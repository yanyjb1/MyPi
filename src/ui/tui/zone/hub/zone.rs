// The subagent hub: this conversation's children, the one under the cursor, and its blocks.

use std::collections::{BTreeMap, HashSet};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::ui::tui::action::{Action, Key, KeyCode, Mods};
use crate::ui::tui::lang::Lang;
use crate::ui::tui::theme::Accent;
use crate::ui::tui::zone::page::Picker;
use crate::ui::tui::zone::{
    Claim, Frame, Kind, Local, Memo, Outcome, Shared, Structure, TermSize, Zone, ZoneId,
};

// No modifiers held.
const PLAIN: Mods = Mods {
    ctrl: false,
    alt: false,
    shift: false,
};

// Ctrl+X: stop what the selected child is running. Rare enough not to be pressed by accident.
const BREAK: Mods = Mods {
    ctrl: true,
    alt: false,
    shift: false,
};

// The display language the page is written in.
const LANG_KEY: &str = crate::ui::tui::lang::Lang::KEY;

// How often a page following a running child reads its list and preview again.
const REFRESH: Duration = Duration::from_secs(2);

// Blocks one page key moves in the preview.
const PAGE: usize = 10;

// Blocks the preview keeps in hand on either side of the cursor.
const SPAN: usize = 40;

// One child, as its row described it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: i64,
    pub name: Option<String>,
    pub first_user: Option<String>,
    pub rounds: i64,
    pub bytes: i64,
    pub cwd: Option<String>,
    // The loop holds this session right now: its task is still running.
    pub running: bool,
}

impl Row {
    fn of(value: &Value) -> Option<Row> {
        Some(Row {
            id: value.get("id")?.as_i64()?,
            name: value
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            first_user: value
                .get("first_user")
                .and_then(Value::as_str)
                .map(str::to_string),
            rounds: value.get("rounds").and_then(Value::as_i64).unwrap_or(0),
            bytes: value.get("bytes").and_then(Value::as_i64).unwrap_or(0),
            cwd: value.get("cwd").and_then(Value::as_str).map(str::to_string),
            running: value
                .get("running")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
    }
}

// One block of the child on show, as an answer carried it.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub id: i64,
    pub kind: String,
    pub text: String,
}

impl Block {
    fn of(value: &Value) -> Option<Block> {
        let id = value.get("id")?.as_i64()?;
        let kind = short_kind(value.get("kind")?.as_str()?);
        let lead = ["content", "text", "name", "args", "result"]
            .iter()
            .filter_map(|key| value.get(*key).and_then(Value::as_str))
            .find(|text| !text.trim().is_empty())
            .unwrap_or("");
        let first = lead.lines().next().unwrap_or("").trim();
        let text = if first.is_empty() {
            format!("{kind}: #{id}")
        } else {
            format!("{kind}: {first}")
        };
        Some(Block {
            id,
            kind: kind.to_string(),
            text,
        })
    }
}

// The wire's block type, in the shorter words a preview row uses.
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

pub struct HubZone {
    shared: Shared,
    size: Option<TermSize>,
    // The conversation whose children this page lists; `None` = nothing to look at.
    session: Option<i64>,
    picker: Picker<Row>,
    // The child the preview belongs to.
    peeked: Option<i64>,
    // That child's structural window: which block ids the conversation holds.
    structure: Structure,
    // That child's blocks, by id, in the order they landed.
    blocks: BTreeMap<i64, Block>,
    // The id ranges asked for and not answered yet.
    asking: HashSet<[i64; 2]>,
    // Whether the keys move the child list or the preview.
    peek: bool,
    // The preview cursor, as a block id.
    cursor: Option<i64>,
    // Whether a list ever arrived: tells "empty" from "not yet".
    loaded: bool,
    // The list itself was refused: the page says why instead of "no subagent".
    error: Option<String>,
    // The language the page is written in; `en` until the config answers.
    lang: Lang,
    // When the list was last asked for.
    asked: Option<Instant>,
    // Bumped whenever this zone's rows would differ.
    rev: u64,
}

impl Default for HubZone {
    fn default() -> Self {
        Self {
            shared: Shared::default(),
            size: None,
            session: None,
            picker: Picker::default(),
            peeked: None,
            structure: Structure::default(),
            blocks: BTreeMap::new(),
            asking: HashSet::new(),
            peek: false,
            cursor: None,
            loaded: false,
            error: None,
            lang: Lang::default(),
            asked: None,
            rev: 0,
        }
    }
}

impl HubZone {
    // Enter the page: forget the last visit, then ask for the list.
    fn begin(&mut self) {
        self.picker.clear();
        self.forget_preview();
        self.loaded = false;
        self.error = None;
        self.list();
        self.shared
            .mail
            .send(Frame::new("call", "config", json!({"key": LANG_KEY})));
    }

    fn forget_preview(&mut self) {
        self.peeked = None;
        self.structure.clear();
        self.blocks.clear();
        self.asking.clear();
        self.peek = false;
        self.cursor = None;
    }

    // The children are refetched on entry and while one of them runs: it is a fact about the outside.
    fn list(&mut self) {
        // The clock turns even with nothing to ask, so an idle page is not asked again every tick.
        self.asked = Some(Instant::now());
        let Some(session) = self.session else {
            return;
        };
        self.shared
            .mail
            .send(Frame::new("call", "children", json!({"session": session})));
    }

    // One child's whole structure plus its newest blocks.
    fn ask_preview(&mut self, id: i64) {
        self.shared
            .mail
            .send(Frame::new("call", "init", json!({"session": id})));
    }

    fn load(&mut self, content: &Value) {
        self.error = None;
        let rows = content
            .get("rows")
            .and_then(Value::as_array)
            .map(|rows| rows.iter().filter_map(Row::of).collect())
            .unwrap_or_default();
        self.picker.replace(rows, |row| title(row).to_lowercase());
        self.loaded = true;
        self.followed();
    }

    // The list did not arrive: the page says why rather than draw the failure as "no children".
    fn refused(&mut self, message: String) {
        self.error = Some(message);
        self.picker.clear_rows();
        self.loaded = true;
    }

    // The preview belongs to the row under the cursor; moving the cursor moves it.
    fn followed(&mut self) {
        let id = self.picker.row().map(|row| row.id);
        if id == self.peeked {
            return;
        }
        let peek = self.peek;
        self.forget_preview();
        self.peeked = id;
        self.peek = peek && id.is_some();
        if let Some(id) = id {
            self.ask_preview(id);
        }
    }

    // The whole conversation's block ids, oldest first.
    fn ids_of(&self) -> Vec<i64> {
        let mut ids: Vec<i64> = self
            .structure
            .turns()
            .flat_map(|turn| turn.ranges.iter())
            .flat_map(|[lo, hi]| *lo..=*hi)
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    }

    fn merge(&mut self, rows: &Value) {
        for row in rows.as_array().into_iter().flatten() {
            if let Some(block) = Block::of(row) {
                self.blocks.insert(block.id, block);
            }
        }
        if self.cursor.is_none() {
            self.cursor = self.ids_of().last().copied();
        }
    }

    // The block the preview's cursor stands on, clamped inside the conversation.
    fn cursor_at(&self) -> Option<i64> {
        let ids = self.ids_of();
        match self.cursor {
            Some(cursor) if ids.contains(&cursor) => Some(cursor),
            _ => ids.last().copied(),
        }
    }

    // Move the preview's cursor by `delta` blocks.
    fn step(&mut self, delta: i64) {
        let ids = self.ids_of();
        if ids.is_empty() {
            return;
        }
        let at = self
            .cursor_at()
            .and_then(|cursor| ids.iter().position(|id| *id == cursor))
            .unwrap_or(ids.len() - 1) as i64;
        let next = (at + delta).clamp(0, ids.len() as i64 - 1) as usize;
        self.cursor = Some(ids[next]);
    }

    fn jump(&mut self, last: bool) {
        let ids = self.ids_of();
        self.cursor = if last {
            ids.last().copied()
        } else {
            ids.first().copied()
        };
    }

    // Ask for the blocks around the cursor that are not in hand yet, one request per screen.
    fn wanted(&mut self) {
        let Some(session) = self.peeked else {
            return;
        };
        let ids = self.ids_of();
        if ids.is_empty() {
            return;
        }
        let at = self
            .cursor_at()
            .and_then(|cursor| ids.iter().position(|id| *id == cursor))
            .unwrap_or(ids.len() - 1);
        let lo = at.saturating_sub(SPAN);
        let hi = (at + SPAN).min(ids.len().saturating_sub(1));

        let mut missing: Vec<i64> = ids[lo..=hi]
            .iter()
            .copied()
            .filter(|id| !self.blocks.contains_key(id))
            .collect();
        missing.sort_unstable();

        // The server answers one structural range at a time: a run never crosses one.
        let mut ranges: Vec<[i64; 2]> = Vec::new();
        for id in missing {
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

    fn bump(&mut self) {
        self.rev = self.rev.wrapping_add(1);
    }

    fn enter(&mut self, size: TermSize, _memo: &Memo) {
        self.size = Some(size);
        self.begin();
    }

    fn follow(&mut self, id: Option<i64>) {
        self.session = id;
        if id.is_none() {
            self.picker.clear();
            self.forget_preview();
            self.loaded = false;
        }
    }

    fn read(&mut self, frame: &Frame) {
        match (frame.kind.as_str(), frame.name.as_str()) {
            ("call", "children") => {
                // Only this page's own answer: another conversation's is not its business.
                if !mine(&frame.payload, self.session) {
                    return;
                }
                match crate::ui::tui::lang::refused(self.lang, &frame.payload) {
                    Some(message) => self.refused(message),
                    None => self.load(&frame.payload),
                }
            }
            // The preview belongs to the child the cursor chose, whatever this page follows.
            ("call", "init") => {
                if frame.payload.get("session").and_then(Value::as_i64) != self.peeked {
                    return;
                }
                if let Some(snapshot) = frame.payload.get("structure") {
                    self.structure = Structure::of(snapshot);
                    self.asking.clear();
                }
                if let Some(rows) = frame.payload.get("blocks") {
                    self.merge(rows);
                }
            }
            ("call", "blocks") => {
                if frame.payload.get("session").and_then(Value::as_i64) != self.peeked {
                    return;
                }
                if let Some(rows) = frame.payload.get("blocks") {
                    self.merge(rows);
                }
                self.asking
                    .retain(|[lo, hi]| !(*lo..=*hi).all(|id| self.blocks.contains_key(&id)));
            }
            ("call", "config") => {
                if frame.payload.get("key").and_then(Value::as_str) == Some(LANG_KEY)
                    && let Some(lang) = frame
                        .payload
                        .get("value")
                        .and_then(Value::as_str)
                        .and_then(Lang::parse)
                {
                    self.lang = lang;
                }
            }
            ("local", "resized") => {
                if let Some(size) = TermSize::of(&frame.payload) {
                    self.size = Some(size);
                }
            }
            _ => {}
        }
    }

    // Stop what the selected child is running. The child stays; only its turn ends.
    fn interrupt(&mut self) {
        let Some(row) = self.picker.row() else {
            return;
        };
        if !row.running {
            return;
        }
        self.shared
            .mail
            .send(Frame::new("post", "interrupt", json!({"session": row.id})));
        self.list();
    }

    fn back(&mut self) -> Outcome {
        Outcome::Show {
            to: ZoneId::Main,
            memo: Vec::new(),
        }
    }

    fn act(&mut self, action: &Action) -> (Outcome, Vec<Local>) {
        match action {
            Action::ScrollUp { shift } => self.vertical(if *shift { -10 } else { -1 }),
            Action::ScrollDown { shift } => self.vertical(if *shift { 10 } else { 1 }),
            Action::Paste(_) => {}
            Action::Key(key) => {
                if key.mods == BREAK && key.code == KeyCode::Char('x') {
                    self.interrupt();
                    return (Outcome::Stay, Vec::new());
                }
                if key.code == KeyCode::Esc {
                    // A reader inside the preview leaves it first; the second Esc leaves the page.
                    if self.peek {
                        self.peek = false;
                        return (Outcome::Stay, Vec::new());
                    }
                    return (self.back(), Vec::new());
                }
                if key.mods != PLAIN {
                    return (Outcome::Stay, Vec::new());
                }
                match key.code {
                    KeyCode::Enter => {
                        if !self.peek && self.peeked.is_some() {
                            self.peek = true;
                        }
                    }
                    KeyCode::Up => self.vertical(-1),
                    KeyCode::Down => self.vertical(1),
                    KeyCode::PageUp => self.vertical(-(PAGE as i64)),
                    KeyCode::PageDown => self.vertical(PAGE as i64),
                    KeyCode::Home => self.edge(false),
                    KeyCode::End => self.edge(true),
                    _ => {}
                }
            }
        }
        (Outcome::Stay, Vec::new())
    }

    // The arrow keys move whichever half holds the attention.
    fn vertical(&mut self, delta: i64) {
        if self.peek {
            self.step(delta);
            self.wanted();
            return;
        }
        self.picker.move_by(delta);
        self.followed();
    }

    fn edge(&mut self, last: bool) {
        if self.peek {
            self.jump(last);
            self.wanted();
            return;
        }
        if last {
            self.picker.last();
        } else {
            self.picker.first();
        }
        self.followed();
    }
}

// Whether a list answer belongs to the conversation this page follows.
fn mine(content: &Value, session: Option<i64>) -> bool {
    match (content.get("session").and_then(Value::as_i64), session) {
        (Some(said), Some(ours)) => said == ours,
        (None, _) => true,
        (Some(_), None) => false,
    }
}

// The first line: a name, else the child's first message, else a label from its id.
pub(crate) fn title(row: &Row) -> String {
    if let Some(name) = &row.name {
        return name.clone();
    }
    if let Some(first) = row.first_user.as_deref() {
        let line = first.lines().next().unwrap_or("").trim();
        if !line.is_empty() {
            return line.to_string();
        }
    }
    format!("subagent #{}", row.id)
}

impl Zone for HubZone {
    fn bind(&mut self, shared: Shared) {
        self.shared = shared;
    }

    fn keys(&self) -> Vec<Claim> {
        let key = |mods, code| Claim::Key(Key { code, mods });
        vec![
            key(PLAIN, KeyCode::Esc),
            key(PLAIN, KeyCode::Enter),
            key(PLAIN, KeyCode::Up),
            key(PLAIN, KeyCode::Down),
            key(PLAIN, KeyCode::PageUp),
            key(PLAIN, KeyCode::PageDown),
            key(PLAIN, KeyCode::Home),
            key(PLAIN, KeyCode::End),
            key(BREAK, KeyCode::Char('x')),
            Claim::Wheel,
            Claim::Paste,
        ]
    }

    fn kinds(&self) -> &'static [Kind] {
        &[Kind::Call]
    }

    fn revision(&self) -> u64 {
        self.rev
    }

    // A running child keeps writing: its list and its preview are read again on this clock.
    fn anim(&self) -> Option<Duration> {
        (self.peek || self.running() > 0).then_some(REFRESH)
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
        let due = self.asked.is_none_or(|at| at.elapsed() >= REFRESH);
        if !due {
            return Vec::new();
        }
        if !self.peek && self.running() == 0 && self.loaded {
            return Vec::new();
        }
        self.list();
        if let Some(id) = self.peeked {
            self.ask_preview(id);
        }
        self.wanted();
        self.bump();
        Vec::new()
    }

    fn event(&mut self, frame: &Frame) -> Vec<Local> {
        self.read(frame);
        self.wanted();
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

impl HubZone {
    pub(crate) fn lang(&self) -> Lang {
        self.lang
    }

    pub(crate) fn size(&self) -> Option<TermSize> {
        self.size
    }

    pub(crate) fn accent(&self) -> Accent {
        self.shared.palette.get()
    }

    pub(crate) fn picker(&self) -> &Picker<Row> {
        &self.picker
    }

    pub(crate) fn picker_mut(&mut self) -> &mut Picker<Row> {
        &mut self.picker
    }

    pub(crate) fn loaded(&self) -> bool {
        self.loaded
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(crate) fn peek(&self) -> bool {
        self.peek
    }

    // The whole conversation's block ids, oldest first.
    pub(crate) fn order(&self) -> Vec<i64> {
        self.ids_of()
    }

    pub(crate) fn block(&self, id: i64) -> Option<&Block> {
        self.blocks.get(&id)
    }

    pub(crate) fn cursor(&self) -> Option<i64> {
        self.cursor_at()
    }

    // How many of the listed children are still on the loop's table.
    pub(crate) fn running(&self) -> usize {
        (0..self.picker.visible_len())
            .filter(|i| self.picker.row_at(*i).is_some_and(|row| row.running))
            .count()
    }
}
