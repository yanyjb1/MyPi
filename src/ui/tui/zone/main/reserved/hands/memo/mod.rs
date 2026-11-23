// The `/memo` hand: the names of both scopes, one name's versions, and the writes.

mod history;
mod keys;
mod list;
mod read;

use ratatui::text::Line;
use serde_json::Value;

use crate::ui::tui::action::{Key, KeyCode};
use crate::ui::tui::lang::Lang;
use crate::ui::tui::text::display_width;
use crate::ui::tui::theme::Accent;
use crate::ui::tui::zone::main::desk::Reply;
use crate::ui::tui::zone::main::desk::{MemoAsk, MemoScope};

use super::super::hand::{Hand, MAX_ROWS, Report};

// One row of the name list; `content` is its newest version.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Item {
    pub(super) scope: MemoScope,
    pub(super) name: String,
    pub(super) content: String,
}

// One saved version, newest last.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Version {
    pub(super) ts: Option<String>,
    pub(super) content: String,
}

// What the hand asks the desk to carry out.
#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    Ask(MemoAsk),
    Copy(String),
}

// The face the panel shows.
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
    Read {
        scope: MemoScope,
        name: String,
        versions: Vec<Version>,
        selected: usize,
        row: usize,
    },
}

// The list keeps a narrow block; the version views take two thirds of the terminal.
const NARROW_LIST: usize = 64;
const NARROW_WIDE: usize = 84;

// The block width and its left margin: two thirds, a floor, never past the terminal.
fn layout(width: usize, narrow: usize) -> (usize, usize) {
    let block = (width * 2 / 3).max(narrow).min(width);
    (block, width.saturating_sub(block) / 2)
}

pub(super) use super::text::{clip, wrap};

// The panel's state.
#[derive(Debug)]
pub struct MemoHand {
    open: bool,
    // Every name in both scopes, decoded from `call memo list`.
    pub(super) items: Vec<Item>,
    // The list answered once; before that the panel draws blank.
    known: bool,
    // The row the list has selected.
    pub(super) selected: usize,
    pub(super) face: Face,
    pub(super) note: Option<String>,
    out: Vec<Out>,
    // The language the notes are said in; `en` until the desk says otherwise.
    lang: Lang,
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
            lang: Lang::default(),
        }
    }
}

impl MemoHand {
    // The block a face draws in: the list keeps a narrow one, the reads a wide one.
    fn narrow(&self) -> usize {
        match self.face {
            Face::List => NARROW_LIST,
            Face::History { .. } | Face::Read { .. } => NARROW_WIDE,
        }
    }

    // A new ask landed. A write leaves the face where it is.
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

    // One `call memo` answer: the list, or one name's versions.
    pub fn read(&mut self, answer: &Value) {
        if let Some(error) = self.error_of(answer) {
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

    // One `post memo` answer: the note, and a fresh list.
    pub fn wrote(&mut self, answer: &Value) {
        if let Some(error) = self.error_of(answer) {
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
        self.note = Some(
            match (self.lang, answer.get("removed").and_then(Value::as_u64)) {
                (Lang::En, Some(0)) => format!("no {scope}/{name}"),
                (Lang::En, Some(_)) => format!("removed {scope}/{name}"),
                (Lang::En, None) => format!("written {scope}/{name}"),
                (Lang::Zh, Some(0)) => format!("没有 {scope}/{name}"),
                (Lang::Zh, Some(_)) => format!("已删除 {scope}/{name}"),
                (Lang::Zh, None) => format!("已写入 {scope}/{name}"),
            },
        );
        self.out.push(Out::Ask(MemoAsk::List));
    }

    pub fn drain(&mut self) -> Vec<Out> {
        std::mem::take(&mut self.out)
    }

    // The language the notes are said in.
    pub fn set_lang(&mut self, lang: Lang) {
        self.lang = lang;
    }

    // The error a refused memo answer carries.
    fn error_of(&self, answer: &Value) -> Option<String> {
        crate::ui::tui::lang::refused(self.lang, answer)
    }

    // The list row under the cursor.
    fn current(&self) -> Option<&Item> {
        self.items.get(self.selected)
    }

    // Move the selection of whichever face is showing, wrapping around.
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
            Face::Read {
                versions,
                selected,
                row,
                ..
            } => {
                let Some(version) = versions.iter().rev().nth(*selected) else {
                    return;
                };
                let len = version.content.lines().count();
                if len == 0 {
                    return;
                }
                *row = (*row as isize + delta).clamp(0, len as isize - 1) as usize;
            }
        }
    }

    // Open the selected name's versions, or the selected version in full.
    fn enter(&mut self) {
        match self.face.clone() {
            Face::List => {
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
            Face::History {
                scope,
                name,
                versions,
                selected,
                ..
            } => {
                self.face = Face::Read {
                    scope,
                    name,
                    versions,
                    selected,
                    row: 0,
                };
            }
            Face::Read { .. } => {}
        }
    }

    // Back one face: the version list, or the list of names.
    fn back(&mut self) {
        match self.face.clone() {
            Face::Read {
                scope,
                name,
                versions,
                selected,
                ..
            } => {
                self.face = Face::History {
                    scope,
                    name,
                    versions,
                    selected,
                    loaded: true,
                };
            }
            Face::History { .. } => self.face = Face::List,
            Face::List => {}
        }
    }

    // The selected version goes to the clipboard, whole.
    fn copy_version(&mut self) {
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
        self.note = Some(match self.lang {
            Lang::En => format!(
                "copied {}/{} version {}",
                scope.as_str(),
                name,
                *selected + 1
            ),
            Lang::Zh => format!("已复制 {}/{} 第 {} 版", scope.as_str(), name, *selected + 1),
        });
        self.out.push(Out::Copy(version.content.clone()));
    }

    // The selected line goes to the clipboard, on its own.
    fn copy_line(&mut self) {
        let Face::Read {
            versions,
            selected,
            row,
            ..
        } = &self.face
        else {
            return;
        };
        let Some(version) = versions.iter().rev().nth(*selected) else {
            return;
        };
        let Some(line) = version.content.lines().nth(*row) else {
            return;
        };
        self.note = Some(match self.lang {
            Lang::En => format!("copied line {}", *row + 1),
            Lang::Zh => format!("已复制第 {} 行", *row + 1),
        });
        self.out.push(Out::Copy(line.to_string()));
    }

    // The selected name goes, every version with it.
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
        // Eight rows whatever the face: the strip shows text, so its height never moves.
        Report {
            rows: MAX_ROWS,
            keys: keys::PANEL,
        }
    }

    fn stays(&self) -> bool {
        self.open
    }

    fn on_key(&mut self, key: &Key) -> Option<Reply> {
        match key.code {
            KeyCode::Esc => {
                if matches!(self.face, Face::List) {
                    self.open = false;
                } else {
                    self.back();
                }
            }
            KeyCode::Enter => {
                if !key.mods.shift {
                    self.enter();
                } else if matches!(self.face, Face::List) {
                    self.delete();
                } else if matches!(self.face, Face::History { .. }) {
                    self.copy_version();
                } else {
                    self.copy_line();
                }
            }
            KeyCode::Up | KeyCode::Char('k') | KeyCode::Char('w') => self.step(-1),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('s') => self.step(1),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Char('d') => self.enter(),
            KeyCode::Left | KeyCode::Char('h') | KeyCode::Char('a') => self.back(),
            _ => {}
        }
        None
    }

    fn draw(&self, width: u16, rows: u16, accent: &Accent) -> Vec<Line<'static>> {
        let body = usize::from(rows).saturating_sub(1);
        let mut out = match self.face {
            Face::List => list::body(self, width, body, accent),
            Face::History { .. } => history::body(self, width, body, accent),
            Face::Read { .. } => read::body(self, width, body, accent),
        };
        out.resize(body, Line::default());
        out.push(note_row(&self.note, width, self.narrow(), accent));
        out
    }

    fn left(&mut self) {
        self.open = false;
        self.out.clear();
    }
}

// The last row: the note centered in the panel's own block, blank when there is none.
fn note_row(note: &Option<String>, width: u16, narrow: usize, accent: &Accent) -> Line<'static> {
    let Some(text) = note else {
        return Line::default();
    };
    let (block, lead) = layout(usize::from(width), narrow);
    let text = clip(text, block);
    let pad = lead + block.saturating_sub(display_width(&text)) / 2;
    Line::from(accent.span(format!("{}{text}", " ".repeat(pad))))
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
        ts: row.get("ts").and_then(Value::as_str).map(str::to_string),
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

// The first line of one text, which is what a row shows.
pub(super) fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or_default()
}

pub(super) use super::super::hand::window_start;
