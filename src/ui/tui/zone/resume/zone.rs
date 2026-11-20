//! The session picker: a full-screen list of stored sessions.

use serde_json::{Value, json};

use crate::ui::tui::action::{Action, Key, KeyCode, Mods};
use crate::ui::tui::zone::{Claim, Frame, Kind, Local, Outcome, Shared, TermSize, Zone, ZoneId};

use super::render;

// The one configuration key this zone reads: a fixed offset like `UTC+8`.
const TZ_KEY: &str = "config.server.platform.timezone";

// The display language a refused answer is read in.
const LANG_KEY: &str = crate::ui::tui::lang::Lang::KEY;

/// No modifiers held.
const PLAIN: Mods = Mods {
    ctrl: false,
    alt: false,
    shift: false,
};

/// Which sessions the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Only sessions that ran under the current directory.
    Current,

    /// Every session on this machine.
    All,
}

impl Scope {
    fn toggled(self) -> Self {
        match self {
            Scope::Current => Scope::All,
            Scope::All => Scope::Current,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Scope::Current => "当前目录",
            Scope::All => "全部目录",
        }
    }
}

/// One stored session, as the answer carried it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: i64,
    pub name: Option<String>,
    pub started_at: String,
    pub cwd: Option<String>,

    /// The first user message, when the answer carries it.
    pub preview: Option<String>,

    /// The session's size, when the answer carries it.
    pub bytes: Option<i64>,

    /// The time of the session's last block; the age counts from it.
    pub last_ts: Option<String>,

    /// How many memos the session holds.
    pub memos: i64,
}

impl Row {
    fn of(value: &Value) -> Option<Row> {
        Some(Row {
            id: value.get("id")?.as_i64()?,
            name: value.get("name")?.as_str().map(str::to_string),
            started_at: value.get("started_at")?.as_str()?.to_string(),
            cwd: value.get("cwd")?.as_str().map(str::to_string),
            preview: value
                .get("first_user")
                .and_then(Value::as_str)
                .map(str::to_string),
            bytes: value.get("bytes").and_then(Value::as_i64),
            last_ts: value
                .get("last_ts")
                .and_then(Value::as_str)
                .map(str::to_string),
            memos: value.get("memos").and_then(Value::as_i64).unwrap_or(0),
        })
    }
}

pub struct ResumeZone {
    shared: Shared,
    size: Option<TermSize>,
    rows: Vec<Row>,

    /// Indices that passed the filter: filtering keeps indices, not rows.
    visible: Vec<usize>,

    /// Row count at the last load: the visible set is recomputed only when it moves.
    seen: usize,
    filter: String,

    /// Position inside `visible`.
    selected: usize,

    /// First visible row (written back by the render).
    scroll: usize,
    scope: Scope,
    cwd: std::path::PathBuf,

    /// Whether a list ever arrived: tells "empty" from "not yet".
    loaded: bool,

    /// The row a Delete armed; Enter confirms, anything else cancels.
    pending_delete: Option<i64>,

    /// The last answer refused something; the render shows it.
    failed: bool,

    /// The list itself was refused: the render says why instead of "no sessions".
    error: Option<String>,

    /// The server's opening word: what the configuration had to say, drawn above the list.
    opening: Option<String>,

    /// The language a refusal is said in; `en` until the config answers.
    lang: crate::ui::tui::lang::Lang,

    /// Bumped whenever this zone's rows would differ.
    rev: u64,
}

impl Default for ResumeZone {
    fn default() -> Self {
        Self {
            shared: Shared::default(),
            size: None,
            rows: Vec::new(),
            visible: Vec::new(),
            seen: usize::MAX,
            filter: String::new(),
            selected: 0,
            scroll: 0,
            scope: Scope::Current,
            cwd: std::path::PathBuf::from("."),
            loaded: false,
            pending_delete: None,
            failed: false,
            error: None,
            opening: None,
            lang: crate::ui::tui::lang::Lang::default(),
            rev: 0,
        }
    }
}

impl ResumeZone {
    /// Enter the page: reset, then ask for the list.
    fn begin(&mut self) {
        self.loaded = false;
        self.scroll = 0;
        self.pending_delete = None;
        self.failed = false;
        self.error = None;
        self.filter.clear();
        self.list();
    }

    /// The list is refetched every time: it is a fact about the outside.
    fn list(&self) {
        let under = self.under();
        self.shared
            .mail
            .send(Frame::new("call", "sessions", json!({"under": under})));
    }

    fn under(&self) -> Option<String> {
        match self.scope {
            Scope::Current => Some(self.cwd.to_string_lossy().to_string()),
            Scope::All => None,
        }
    }

    fn load(&mut self, content: &Value) {
        self.error = None;
        let rows = content
            .get("rows")
            .and_then(Value::as_array)
            .map(|rows| rows.iter().filter_map(Row::of).collect())
            .unwrap_or_default();
        self.rows = rows;
        self.loaded = true;
        self.seen = self.rows.len();
        self.refilter();
    }

    /// The list did not arrive: say so rather than draw the failure as an empty directory.
    fn refused(&mut self, message: String) {
        self.error = Some(format!("会话列表读取失败：{message}"));
        self.rows.clear();
        self.visible.clear();
        self.seen = 0;
        self.selected = 0;
        self.scroll = 0;
        self.loaded = true;
    }

    /// The server's opening word: the configuration's warnings, kept until the page is left.
    fn arrived(&mut self, content: &Value) {
        let Some(notices) = content.get("notices").and_then(Value::as_array) else {
            return;
        };
        let lines: Vec<String> = notices
            .iter()
            .filter_map(|notice| notice.get("text").and_then(Value::as_str))
            .map(str::to_string)
            .collect();
        if !lines.is_empty() {
            self.opening = Some(lines.join("；"));
        }
    }

    fn row(&self) -> Option<&Row> {
        self.rows.get(*self.visible.get(self.selected)?)
    }

    fn selected_id(&self) -> Option<i64> {
        self.row().map(|row| row.id)
    }

    /// Recompute the visible set. The filter matches what the answer carries.
    fn refilter(&mut self) {
        let needle = self.filter.to_lowercase();
        self.visible = self
            .rows
            .iter()
            .enumerate()
            .filter(|(_, row)| needle.is_empty() || haystack(row).contains(&needle))
            .map(|(i, _)| i)
            .collect();
        self.selected = self.selected.min(self.visible.len().saturating_sub(1));
        self.scroll = 0;
    }

    fn move_by(&mut self, delta: i64) {
        if self.visible.is_empty() {
            return;
        }
        let last = self.visible.len() - 1;
        let next = self.selected as i64 + delta;
        self.selected = next.clamp(0, last as i64) as usize;
    }

    fn bump(&mut self) {
        self.rev = self.rev.wrapping_add(1);
    }

    fn enter(&mut self, size: TermSize, memo: &crate::ui::tui::zone::Memo) {
        self.size = Some(size);
        if let Some(cwd) = memo.read("cwd") {
            self.cwd = cwd.into();
        }
        self.begin();
        for key in [TZ_KEY, LANG_KEY] {
            self.shared
                .mail
                .send(Frame::new("call", "config", json!({"key": key})));
        }
    }

    fn follow(&mut self, id: Option<i64>) {
        if id.is_none() {
            self.rows.clear();
            self.visible.clear();
            self.seen = usize::MAX;
            self.filter.clear();
            self.selected = 0;
            self.scroll = 0;
            self.loaded = false;
            self.pending_delete = None;
        }
    }

    fn read(&mut self, frame: &Frame) {
        match (frame.kind.as_str(), frame.name.as_str()) {
            ("call", "sessions") => match self.error_of(&frame.payload) {
                Some(message) => self.refused(message),
                None => self.load(&frame.payload),
            },
            ("post", "delete") => self.failed = frame.payload.get("error").is_some(),
            ("call", "config") => {
                let value = frame.payload.get("value").and_then(Value::as_str);
                match frame.payload.get("key").and_then(Value::as_str) {
                    Some(LANG_KEY) => {
                        if let Some(lang) = value.and_then(crate::ui::tui::lang::Lang::parse) {
                            self.lang = lang;
                        }
                    }
                    _ => {
                        if let Some(value) = value {
                            render::set_local_offset(value);
                        }
                    }
                }
            }
            ("local", "opened") => self.arrived(&frame.payload),
            _ => {}
        }
    }

    fn act(&mut self, action: &Action) -> (Outcome, Vec<Local>) {
        let stay = (Outcome::Stay, Vec::new());
        let back = (
            Outcome::Show {
                to: ZoneId::Main,
                memo: Vec::new(),
            },
            Vec::new(),
        );
        match action {
            Action::ScrollUp { .. } => self.move_by(-1),
            Action::ScrollDown { .. } => self.move_by(1),
            Action::Paste(text) => {
                // A paste is a filter word here: a picker is not worth multi-line semantics.
                self.filter.push_str(&text.replace('\n', " "));
                self.refilter();
            }
            Action::Key(key) => {
                let ctrl = key.mods.ctrl;
                if !matches!(key.code, KeyCode::Enter | KeyCode::Delete | KeyCode::Esc) {
                    self.pending_delete = None;
                }
                match key.code {
                    KeyCode::Delete => {
                        self.pending_delete = self.selected_id();
                        return stay;
                    }
                    KeyCode::Esc => return back,
                    KeyCode::Enter => {
                        let Some(id) = self.selected_id() else {
                            return stay;
                        };
                        if self.pending_delete == Some(id) {
                            self.failed = false;
                            self.shared.mail.send(Frame::new(
                                "post",
                                "delete",
                                json!({"session": id}),
                            ));
                            self.list();
                            return stay;
                        }
                        return (
                            Outcome::Show {
                                to: ZoneId::Main,
                                memo: Vec::new(),
                            },
                            vec![Local::Attach(id)],
                        );
                    }
                    KeyCode::Up => self.move_by(-1),
                    KeyCode::Down => self.move_by(1),
                    KeyCode::PageUp => self.move_by(-PAGE),
                    KeyCode::PageDown => self.move_by(PAGE),
                    KeyCode::Home => self.selected = 0,
                    KeyCode::End => self.selected = self.visible.len().saturating_sub(1),
                    KeyCode::Tab => {
                        self.scope = self.scope.toggled();
                        self.list();
                        return stay;
                    }
                    KeyCode::Backspace => {
                        self.filter.pop();
                        self.refilter();
                    }
                    KeyCode::Char('u') if ctrl => {
                        self.filter.clear();
                        self.refilter();
                    }
                    KeyCode::Char(c) if !ctrl => {
                        self.filter.push(c);
                        self.refilter();
                    }
                    _ => {}
                }
            }
        }
        stay
    }
}

// The error a refused answer carries, in the language this zone reads.
impl ResumeZone {
    fn error_of(&self, answer: &Value) -> Option<String> {
        crate::ui::tui::lang::refused(self.lang, answer)
    }
}

/// Everything about one session the filter can match on.
pub(crate) fn haystack(row: &Row) -> String {
    format!(
        "{} {} {} {}",
        row.id,
        row.name.as_deref().unwrap_or(""),
        row.preview.as_deref().unwrap_or(""),
        row.cwd.as_deref().unwrap_or("")
    )
    .to_lowercase()
}

/// The first line: an explicit name, else the first user message, else a synthesized label.
pub(crate) fn title(row: &Row) -> String {
    if let Some(name) = &row.name {
        return name.clone();
    }
    if let Some(first) = preview_text(row) {
        return first;
    }
    display_name(row)
}

/// The preview line: only named sessions draw their first message (it is the title otherwise).
pub(crate) fn preview(row: &Row) -> Option<String> {
    row.name.as_ref()?;
    preview_text(row)
}

fn preview_text(row: &Row) -> Option<String> {
    let text = row.preview.as_deref()?;
    let first = text.lines().next().unwrap_or("").trim();
    (!first.is_empty()).then(|| first.to_string())
}

/// `MM-DD:HH-MMSS+first 7 chars`, the name an unnamed session is shown under.
fn display_name(row: &Row) -> String {
    let prefix: String = row
        .preview
        .as_deref()
        .map(|text| text.chars().take(7).collect())
        .unwrap_or_default();
    let t: Vec<char> = row.started_at.chars().collect();
    let part = |a: usize, b: usize| t[a..b].iter().collect::<String>();
    if t.len() >= 19 {
        format!(
            "{}-{}:{}-{}{}+{}",
            part(5, 7),
            part(8, 10),
            part(11, 13),
            part(14, 16),
            part(17, 19),
            prefix
        )
    } else {
        row.started_at.clone()
    }
}

/// Rows per page.
const PAGE: i64 = 8;

impl Zone for ResumeZone {
    fn bind(&mut self, shared: Shared) {
        self.shared = shared;
    }

    fn keys(&self) -> Vec<Claim> {
        let key = |code| Claim::Key(Key { code, mods: PLAIN });
        vec![
            key(KeyCode::Esc),
            key(KeyCode::Enter),
            key(KeyCode::Delete),
            key(KeyCode::Up),
            key(KeyCode::Down),
            key(KeyCode::PageUp),
            key(KeyCode::PageDown),
            key(KeyCode::Home),
            key(KeyCode::End),
            key(KeyCode::Tab),
            key(KeyCode::Backspace),
            Claim::AnyChar,
            Claim::Paste,
            Claim::Wheel,
            Claim::WheelShift,
        ]
    }

    fn kinds(&self) -> &'static [Kind] {
        &[Kind::Call]
    }

    fn revision(&self) -> u64 {
        self.rev
    }

    fn start(&mut self, size: TermSize, memo: &crate::ui::tui::zone::Memo) -> Vec<Local> {
        self.enter(size, memo);
        self.bump();
        Vec::new()
    }

    fn session(&mut self, id: Option<i64>) -> Vec<Local> {
        self.follow(id);
        self.bump();
        Vec::new()
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

impl ResumeZone {
    pub(crate) fn scope(&self) -> Scope {
        self.scope
    }

    pub(crate) fn scope_label(&self) -> &'static str {
        self.scope.label()
    }

    pub(crate) fn filter(&self) -> &str {
        &self.filter
    }

    pub(crate) fn pending_delete(&self) -> Option<i64> {
        self.pending_delete
    }

    pub(crate) fn loaded(&self) -> bool {
        self.loaded
    }

    pub(crate) fn failed(&self) -> bool {
        self.failed
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub(crate) fn opening(&self) -> Option<&str> {
        self.opening.as_deref()
    }

    pub(crate) fn visible_len(&self) -> usize {
        self.visible.len()
    }

    pub(crate) fn selected(&self) -> usize {
        self.selected
    }

    pub(crate) fn set_scroll(&mut self, top: usize) {
        self.scroll = top;
    }

    pub(crate) fn scroll(&self) -> usize {
        self.scroll
    }

    pub(crate) fn size(&self) -> Option<TermSize> {
        self.size
    }

    /// Row `i` of the visible list.
    pub(crate) fn row_at(&self, i: usize) -> Option<&Row> {
        let k = *self.visible.get(i)?;
        self.rows.get(k)
    }
}
