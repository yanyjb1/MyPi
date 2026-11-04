//! The block tree: a full-screen navigator over the blocks of one conversation.

use std::collections::BTreeMap;

use serde_json::{Value, json};

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
        let lead = row
            .get("content")
            .or_else(|| row.get("text"))
            .or_else(|| row.get("result"))
            .or_else(|| row.get("args"))
            .and_then(Value::as_str)
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
    /// The live accent, broadcast as a local frame.
    accent: Accent,
    /// The rows are stale and must be rebuilt before the next draw.
    stale: bool,
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
            accent: Accent::default(),
            stale: true,
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
        if let Some(id) = keep
            && let Some(at) = self.rows.iter().position(|row| row.block_id == id)
        {
            self.selected = at;
        } else {
            self.selected = self.selected.min(self.rows.len().saturating_sub(1));
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

    fn current(&self) -> usize {
        let Some(last) = self.blocks.keys().next_back() else {
            return 0;
        };
        self.rows
            .iter()
            .position(|row| row.block_id == *last)
            .unwrap_or(0)
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
            memo.push((
                "tree.focus".to_string(),
                format!("{}:{}", row.turn, row.block_id),
            ));
        }
        Outcome::Show {
            to: ZoneId::Main,
            memo,
        }
    }
}

impl Zone for TreeZone {
    fn keys(&self) -> Vec<Claim> {
        let key = |code| Claim::Key(Key { code, mods: PLAIN });
        vec![
            key(KeyCode::Up),
            key(KeyCode::Down),
            key(KeyCode::PageUp),
            key(KeyCode::PageDown),
            key(KeyCode::Home),
            key(KeyCode::End),
            key(KeyCode::Enter),
            key(KeyCode::Esc),
            key(KeyCode::Char('q')),
            key(KeyCode::Char('s')),
        ]
    }

    fn kinds(&self) -> &'static [Kind] {
        &[Kind::Call]
    }

    fn start(&mut self, size: TermSize, _memo: &Memo) -> Vec<Request> {
        self.size = Some(size);
        self.stale = true;
        self.sync();
        self.selected = self.current();
        vec![Request::Structure, Request::call("init", json!({}))]
    }

    fn session(&mut self, id: Option<i64>) -> Vec<Request> {
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

    fn event(&mut self, frame: &Frame) -> Vec<Request> {
        match (frame.class.as_str(), frame.name.as_str()) {
            ("call", "init") | ("call", "blocks") => {
                if let Some(rows) = frame.content.get("blocks") {
                    self.merge(rows);
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

    fn action(&mut self, action: &Action) -> (Outcome, Vec<Request>) {
        self.sync();
        let stay = (Outcome::Stay, Vec::new());
        match action {
            Action::ScrollUp { .. } => self.move_by(-1),
            Action::ScrollDown { .. } => self.move_by(1),
            Action::Paste(_) => {}
            Action::Key(key) => match key.code {
                KeyCode::Up => self.move_by(-1),
                KeyCode::Down => self.move_by(1),
                KeyCode::PageUp => self.move_by(-10),
                KeyCode::PageDown => self.move_by(10),
                KeyCode::Home => self.selected = 0,
                KeyCode::End => self.selected = self.rows.len().saturating_sub(1),
                KeyCode::Char('s') if !key.mods.ctrl && !key.mods.alt => {
                    self.only_user = !self.only_user;
                    self.stale = true;
                    self.sync();
                }
                KeyCode::Enter => {
                    let rewind = !key.mods.shift;
                    if rewind {
                        if let Some(row) = self.rows.get(self.selected) {
                            let request = Request::post(
                                "rewind",
                                json!({"turn": row.turn, "block": row.block_id}),
                            );
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
        stay
    }

    fn structure(&mut self, snapshot: &Turns) -> Vec<Request> {
        self.structure = snapshot.clone();
        self.stale = true;
        Vec::new()
    }

    fn render(&mut self) -> Vec<ratatui::text::Line<'static>> {
        self.sync();
        super::render::rows(
            &self.rows,
            self.selected,
            self.size,
            &mut self.scroll,
            &self.accent,
        )
    }
}
