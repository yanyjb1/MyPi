//! The `/memo` hand: the names of both scopes, one name's versions, and the writes.

mod history;
mod keys;
mod list;

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use serde_json::Value;

use crate::ui::tui::action::{Key, KeyCode};
use crate::ui::tui::theme::{Accent, fixed};
use crate::ui::tui::zone::main::desk::Reply;
use crate::ui::tui::zone::{MemoAsk, MemoScope};

use super::super::hand::{Hand, Report};

/// One row of the name list; `content` is its newest version.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Item {
    pub(super) scope: MemoScope,
    pub(super) name: String,
    pub(super) content: String,
}

/// One saved version, newest last.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Version {
    pub(super) content: String,
}

/// What the hand asks the desk to carry out.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    Ask(MemoAsk),
    Copy(String),
}

/// The face the panel shows.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Face {
    List,
    History {
        scope: MemoScope,
        name: String,
        versions: Vec<Version>,
        selected: usize,
        loaded: bool,
    },
}

/// The panel's state.
#[derive(Debug)]
pub struct MemoHand {
    open: bool,
    /// Every name in both scopes; the list's height comes from here.
    pub(super) items: Vec<Item>,
    /// The list answered once: the pane may freeze its height.
    known: bool,
    /// The row the list has selected.
    pub(super) selected: usize,
    pub(super) face: Face,
    pub(super) note: Option<String>,
    out: Vec<Out>,
}

impl Default for MemoHand {
    fn default() -> Self {
        Self {
            open: false,
            items: Vec::new(),
            known: false,
            selected: 0,
            face: Face::List,
            note: None,
            out: Vec::new(),
        }
    }
}

impl MemoHand {
    /// A new ask landed. A write leaves the face where it is.
    pub fn ask(&mut self, ask: &MemoAsk) {
        self.note = None;
        match ask {
            MemoAsk::List => self.face = Face::List,
            MemoAsk::History { scope, name } => {
                self.face = Face::History {
                    scope: *scope,
                    name: name.clone(),
                    versions: Vec::new(),
                    selected: 0,
                    loaded: false,
                };
            }
            MemoAsk::Add { .. } | MemoAsk::Mod { .. } | MemoAsk::Del { .. } => {}
        }
    }

    /// One `call memo` answer: the list, or one name's versions.
    pub fn read(&mut self, answer: &Value) {
        if let Some(error) = error_of(answer) {
            self.note = Some(error);
            self.known = true;
            return;
        }
        if let Some(memos) = answer.get("memos").and_then(Value::as_array) {
            self.items = memos.iter().filter_map(item_of).collect();
            self.known = true;
            self.selected = self.selected.min(self.items.len().saturating_sub(1));
            return;
        }
        if let Some(rows) = answer.get("versions").and_then(Value::as_array)
            && let Face::History {
                versions,
                selected,
                loaded,
                ..
            } = &mut self.face
        {
            *versions = rows.iter().filter_map(version_of).collect();
            *selected = 0;
            *loaded = true;
        }
    }

    /// One `post memo` answer: the note, and a fresh list.
    pub fn wrote(&mut self, answer: &Value) {
        if let Some(error) = error_of(answer) {
            self.note = Some(error);
            return;
        }
        let scope = answer
            .get("scope")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let name = answer
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        self.note = Some(match answer.get("removed").and_then(Value::as_u64) {
            Some(0) => format!("没有 {scope}/{name}"),
            Some(_) => format!("已删除 {scope}/{name}"),
            None => format!("已写入 {scope}/{name}"),
        });
        self.out.push(Out::Ask(MemoAsk::List));
    }

    /// Take what the hand asked for.
    pub fn drain(&mut self) -> Vec<Out> {
        std::mem::take(&mut self.out)
    }

    /// The list row under the cursor.
    fn current(&self) -> Option<&Item> {
        self.items.get(self.selected)
    }

    /// Move the selection of whichever face is showing, wrapping around.
    fn step(&mut self, delta: isize) {
        match &mut self.face {
            Face::List => {
                if self.items.is_empty() {
                    return;
                }
                let len = self.items.len() as isize;
                self.selected = (self.selected as isize + delta).rem_euclid(len) as usize;
            }
            Face::History {
                versions, selected, ..
            } => {
                if versions.is_empty() {
                    return;
                }
                let len = versions.len() as isize;
                *selected = (*selected as isize + delta).rem_euclid(len) as usize;
            }
        }
    }

    /// Open the selected name's versions.
    fn enter(&mut self) {
        if !matches!(self.face, Face::List) {
            return;
        }
        let Some(item) = self.current() else {
            return;
        };
        let (scope, name) = (item.scope, item.name.clone());
        self.face = Face::History {
            scope,
            name: name.clone(),
            versions: Vec::new(),
            selected: 0,
            loaded: false,
        };
        self.out.push(Out::Ask(MemoAsk::History { scope, name }));
    }

    /// Back to the list.
    fn back(&mut self) {
        if matches!(self.face, Face::History { .. }) {
            self.face = Face::List;
        }
    }

    /// The selected version goes to the clipboard.
    fn copy(&mut self) {
        let Face::History {
            scope,
            name,
            versions,
            selected,
            ..
        } = &self.face
        else {
            return;
        };
        let Some(version) = versions.iter().rev().nth(*selected) else {
            return;
        };
        self.note = Some(format!(
            "已复制 {}/{} 第 {} 版",
            scope.as_str(),
            name,
            *selected + 1
        ));
        self.out.push(Out::Copy(version.content.clone()));
    }

    /// The selected name goes, every version with it.
    fn delete(&mut self) {
        if !matches!(self.face, Face::List) {
            return;
        }
        let Some(item) = self.current() else {
            return;
        };
        self.out.push(Out::Ask(MemoAsk::Del {
            scope: item.scope,
            name: item.name.clone(),
        }));
    }
}

impl Hand for MemoHand {
    fn arrived(&mut self) {
        self.open = true;
    }

    fn report(&self) -> Report {
        Report {
            rows: 1 + self.items.len() as u16 + 2,
            keys: keys::PANEL,
        }
    }

    fn ready(&self) -> bool {
        self.known
    }

    fn stays(&self) -> bool {
        self.open
    }

    fn on_key(&mut self, key: &Key) -> Option<Reply> {
        if key.code == KeyCode::Esc {
            self.open = false;
            return None;
        }
        if key.code == KeyCode::Enter {
            match key.mods.shift {
                true => self.delete(),
                false => match matches!(self.face, Face::List) {
                    true => self.enter(),
                    false => self.copy(),
                },
            }
            return None;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('w') => self.step(-1),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('s') => self.step(1),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('d') => self.enter(),
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('a') => self.back(),
            _ => {}
        }
        None
    }

    fn draw(&self, width: u16, rows: u16, accent: &Accent) -> Vec<Line<'static>> {
        let rows = usize::from(rows);
        let body = rows.saturating_sub(2);
        if body == 0 {
            // Not frozen yet: one line while the list is on its way.
            return vec![Line::from(Span::styled(
                "正在读取…",
                Style::new().fg(fixed::MUTED),
            ))];
        }
        let mut out = vec![Line::default()];
        out.extend(match self.face {
            Face::List => list::body(self, width, body, accent),
            Face::History { .. } => history::body(self, width, body, accent),
        });
        out.resize(1 + body, Line::default());
        out.push(match &self.note {
            Some(text) => Line::from(accent.span(text.clone())),
            None => Line::default(),
        });
        out
    }

    fn left(&mut self) {
        self.open = false;
        self.out.clear();
    }
}

/// The error a refused door carries.
fn error_of(answer: &Value) -> Option<String> {
    let error = answer.get("error")?;
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("memo 操作失败");
    Some(message.to_string())
}

fn item_of(row: &Value) -> Option<Item> {
    Some(Item {
        scope: scope_of(row.get("scope")?.as_str()?)?,
        name: row.get("name")?.as_str()?.to_string(),
        content: row
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}

fn version_of(row: &Value) -> Option<Version> {
    Some(Version {
        content: row
            .get("content")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    })
}

fn scope_of(text: &str) -> Option<MemoScope> {
    match text {
        "current" => Some(MemoScope::Current),
        "global" => Some(MemoScope::Global),
        _ => None,
    }
}

/// The first line of one text, which is what a row shows.
pub(super) fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

pub(super) use super::super::hand::window_start;
