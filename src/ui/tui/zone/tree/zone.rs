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
use crate::ui::tui::zone::page::Picker;

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
    picker: Picker<Row>,
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
            picker: Picker::default(),
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
        let lang = self.lang;
        let keep = self.row().map(|row| row.block_id);
        let rows = super::source::rows(&self.structure, &self.blocks, self.only_user);
        self.picker
            .replace(rows, |row| super::render::haystack(row, lang));
        let at = self
            .landed
            .then(|| keep.and_then(|id| self.position_of(id)))
            .flatten();
        if let Some(index) = at {
            self.picker.select(index);
        } else if !self.landed && self.picker.visible_len() > 0 {
            // The newest block is only known once the blocks themselves are here.
            self.picker.last();
            self.landed = true;
        }
        let last = self.picker.visible_len().saturating_sub(1);
        self.picker.set_scroll(self.picker.scroll().min(last));
    }

    fn position_of(&self, id: i64) -> Option<usize> {
        self.picker.position(|row| row.block_id == id)
    }

    // The rows the body may take, the header and the hints excluded.
    fn body(&self) -> usize {
        let size = self.size.unwrap_or(TermSize { cols: 80, rows: 24 });
        let hints = super::render::hints(self.lang, self.cherish, self.only_user);
        super::render::body_rows(size, &hints)
    }

    // Ask for the blocks the window shows and does not hold yet.
    fn wanted(&mut self) {
        if self.picker.visible_len() == 0 {
            return;
        }
        let body = self.body().max(1);
        self.picker.settle_scroll(body);
        let from = self.picker.scroll().min(self.picker.visible_len());
        let to = (from + body).min(self.picker.visible_len());
        let mut ids: Vec<i64> = (from..to)
            .filter_map(|i| self.picker.row_at(i))
            .map(|row| row.block_id)
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

    fn row(&self) -> Option<&Row> {
        self.picker.row()
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
            self.picker.clear();
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
        let lang = self.lang;
        let haystack = move |row: &Row| super::render::haystack(row, lang);
        match action {
            Action::ScrollUp { shift } => self.picker.move_by(if *shift { -10 } else { -1 }),
            Action::ScrollDown { shift } => self.picker.move_by(if *shift { 10 } else { 1 }),
            Action::Paste(text) => self
                .picker
                .edit_filter(|filter| filter.push_str(&text.replace('\n', " ")), haystack),
            Action::Key(key) => match key.code {
                KeyCode::Up => self.picker.move_by(-1),
                KeyCode::Down => self.picker.move_by(1),
                KeyCode::PageUp => self.picker.move_by(-10),
                KeyCode::PageDown => self.picker.move_by(10),
                KeyCode::Home => self.picker.first(),
                KeyCode::End => self.picker.last(),
                KeyCode::Tab => {
                    self.only_user = !self.only_user;
                    self.stale = true;
                    self.picker.set_scroll(0);
                    self.sync();
                }
                KeyCode::Enter => {
                    let rewind = key.mods.shift == self.cherish;
                    if rewind {
                        if let Some(row) = self.picker.row()
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
                KeyCode::Backspace => self.picker.edit_filter(
                    |f| {
                        f.pop();
                    },
                    haystack,
                ),
                KeyCode::Char('u') if key.mods.ctrl => {
                    self.picker.edit_filter(String::clear, haystack)
                }
                KeyCode::Char(c) if !key.mods.ctrl => {
                    self.picker.edit_filter(|f| f.push(c), haystack)
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
        let before = (self.picker.selected(), self.picker.scroll());
        self.sync();
        self.picker.settle_scroll(self.body());
        if (self.picker.selected(), self.picker.scroll()) != before {
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

    pub(crate) fn picker(&self) -> &Picker<Row> {
        &self.picker
    }

    pub(crate) fn picker_mut(&mut self) -> &mut Picker<Row> {
        &mut self.picker
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

    pub(crate) fn size(&self) -> Option<TermSize> {
        self.size
    }

    pub(crate) fn accent(&self) -> Accent {
        self.shared.palette.get()
    }
}
