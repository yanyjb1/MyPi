//! The input zone: editor, history, statusline, submit, command routing.
//! It does not know what sits above it; it only knows when it lent its keys away.

pub mod command;
mod ctx;
mod editor;
mod queue;
mod render;
mod route;
mod semantics;
mod statusline;

pub use ctx::{Ctx, Data};

use super::desk::{Hire, Reply};
use super::geometry;
use super::pane::Pane;
use crate::ui::tui::action::{Action, Key, KeyCode, Mods};
use crate::ui::tui::theme::Accent;
use crate::ui::tui::zone::{Claim, Frame, Request, Shout, SubZone, TermSize, name};
use serde_json::Value;

use editor::{Editor, Effect, History};
use queue::{Outgoing, Queue};
use statusline::{Setup, StatusLine, StatusTheme};

pub use ctx::{Bill, Prices};

/// What the editor declares when it cannot act on its own.
///
/// Two un-lent Esc this close together ask for the node tree.
pub const ESC_TREE_WINDOW: std::time::Duration = std::time::Duration::from_millis(300);

/// No modifiers held.
const PLAIN: Mods = Mods {
    ctrl: false,
    alt: false,
    shift: false,
};

/// The git segment's scan interval, in seconds.
const GIT_SCAN_KEY: &str = "config.ui.tui.zone.main.input.deco.gitScanInterval";

/// The model a draft opens with; a session row overrides it once there is one.
const DEFAULT_MODEL_KEY: &str = "config.server.api.defaultModel";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitRequest {
    /// Submit the current input, whole.
    Submit(String),
    /// Interrupt the in-flight round.
    Interrupt,
    /// Two un-lent Esc inside [`ESC_TREE_WINDOW`] ask for the node tree.
    OpenTree,
    /// `/resume`: the picker is the app's to show.
    OpenResume,
    /// Quit the program.
    Quit,
    /// `/new`: reset the conversation without attaching another.
    New,
}

/// The input sub-zone. Its height comes from its own content.
pub struct InputZone {
    /// Input viewport start (wrapped row). Independent of the cursor.
    pub scroll: usize,
    /// The height it declares: editor rows + statusline + border.
    pub rows: u16,
    /// The editor itself: text, cursor, undo.
    pub editor: Editor,
    /// Input history: an in-memory ring, 20 entries, never stored.
    pub history: History,
    /// Idle and rewinding: the box is empty, so ↑↓ walks the history.
    pub finished: bool,
    /// The cursor sits in a trigger word: the condition for lending keys.
    pub hang: bool,
    /// The hand to hire, produced here and taken by the zone.
    pending_hire: Option<Hire>,
    /// The last ↑↓ goal column, carried for `Editor::up` / `down`.
    pub goal_col: Option<usize>,
    /// Exit requests, in order: taken by the main zone.
    pending_exits: Vec<ExitRequest>,
    /// Sentences sent while a round was running, head first.
    queue: Queue,
    /// The sentence a submit put out, kept only until its reply says it opened nothing.
    in_flight: Option<String>,
    /// The last slot-opened turn this queue has accounted for.
    last_mailbox_turn: Option<i64>,
    /// When the last un-lent Esc fired: another one inside the window opens the tree.
    esc_armed: Option<std::time::Instant>,
    /// When Enter was last pressed mid-round: another one inside the window submits.
    enter_armed: Option<std::time::Instant>,
    /// The cursor's absolute terminal column, border included.
    pub cursor_col: u16,
    /// The cursor's row inside the container (0 = the statusline row).
    pub cursor_row: u16,
    /// The statusline: the top edge of the container and its own deco table.
    pub statusline: StatusLine,
    /// What the decos were built with; the git scan interval lands here once.
    setup: Setup,
    /// The startup questions were asked; one answer per run is enough.
    booted: bool,
    /// The terminal size from this round's arbitration.
    term: TermSize,
    /// What the parts read.
    data: Data,
    /// Events this zone puts out.
    shouts: Vec<Shout>,
    /// Frames this zone owes the session.
    outbox: Vec<Request>,
}

impl Default for InputZone {
    fn default() -> Self {
        Self {
            scroll: 0,
            rows: 0,
            editor: Editor::new(),
            history: History::new(),
            finished: true,
            hang: false,
            pending_hire: None,
            goal_col: None,
            pending_exits: Vec::new(),
            queue: Queue::default(),
            in_flight: None,
            last_mailbox_turn: None,
            esc_armed: None,
            enter_armed: None,
            cursor_col: geometry::BORDER_COLS as u16,
            cursor_row: 1,
            statusline: StatusLine::new(&Setup::default()),
            setup: Setup::default(),
            booted: false,
            term: TermSize { cols: 80, rows: 24 },
            data: Data::default(),
            shouts: Vec::new(),
            outbox: Vec::new(),
        }
    }
}

impl InputZone {
    /// Everything the editor acts on. `AnyChar` covers the plain typing keys; the rest are
    /// named so no key is taken by accident.
    pub const CLAIMS: &'static [Claim] = &[
        Claim::AnyChar,
        Claim::Paste,
        Claim::Key(Key {
            code: KeyCode::Enter,
            mods: PLAIN,
        }),
        Claim::Key(Key {
            code: KeyCode::Enter,
            mods: Mods {
                ctrl: false,
                alt: true,
                shift: false,
            },
        }),
        Claim::Key(Key {
            code: KeyCode::Enter,
            mods: Mods {
                ctrl: false,
                alt: false,
                shift: true,
            },
        }),
        Claim::Key(Key {
            code: KeyCode::Esc,
            mods: PLAIN,
        }),
        Claim::Key(Key {
            code: KeyCode::Up,
            mods: Mods {
                ctrl: false,
                alt: true,
                shift: false,
            },
        }),
        Claim::Key(Key {
            code: KeyCode::Tab,
            mods: PLAIN,
        }),
        Claim::Key(Key {
            code: KeyCode::Backspace,
            mods: PLAIN,
        }),
        Claim::Key(Key {
            code: KeyCode::Delete,
            mods: PLAIN,
        }),
        Claim::Key(Key {
            code: KeyCode::Left,
            mods: PLAIN,
        }),
        Claim::Key(Key {
            code: KeyCode::Right,
            mods: PLAIN,
        }),
        Claim::Key(Key {
            code: KeyCode::Up,
            mods: PLAIN,
        }),
        Claim::Key(Key {
            code: KeyCode::Down,
            mods: PLAIN,
        }),
        Claim::Key(Key {
            code: KeyCode::Home,
            mods: PLAIN,
        }),
        Claim::Key(Key {
            code: KeyCode::End,
            mods: PLAIN,
        }),
        Claim::Key(Key {
            code: KeyCode::Left,
            mods: Mods {
                ctrl: true,
                alt: false,
                shift: false,
            },
        }),
        Claim::Key(Key {
            code: KeyCode::Right,
            mods: Mods {
                ctrl: true,
                alt: false,
                shift: false,
            },
        }),
        Claim::Key(Key {
            code: KeyCode::Home,
            mods: Mods {
                ctrl: true,
                alt: false,
                shift: false,
            },
        }),
        Claim::Key(Key {
            code: KeyCode::End,
            mods: Mods {
                ctrl: true,
                alt: false,
                shift: false,
            },
        }),
    ];

    /// Drop one session's readings; the workspace stays: it is where the app runs.
    fn restart_data(&mut self) {
        let cwd = std::mem::take(&mut self.data.cwd);
        self.data = Data::default();
        self.data.cwd = cwd;
    }

    /// First frame: the two facts a draft can ask for on its own.
    pub fn boot(&mut self) {
        if self.booted {
            return;
        }
        self.booted = true;
        self.outbox.push(Request::Config {
            key: GIT_SCAN_KEY.to_string(),
        });
        self.outbox.push(Request::Config {
            key: DEFAULT_MODEL_KEY.to_string(),
        });
    }

    /// The hand the editor asked for, if it asked this frame.
    pub fn take_hire(&mut self) -> Option<Hire> {
        self.pending_hire.take()
    }

    /// Frames this zone asked the session for.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.outbox)
    }

    /// The exits waiting to be turned into requests.
    pub fn take_exits(&mut self) -> Vec<ExitRequest> {
        std::mem::take(&mut self.pending_exits)
    }

    /// The editor's text and where the cursor sits in it.
    pub fn draft(&self) -> (String, usize) {
        (self.editor.text(), self.editor.cursor())
    }

    /// The height it asks for: content rows, capped at a share of the terminal.
    fn request_height_for(&self, wrapped_lines: usize, term_h: u16) -> u16 {
        geometry::container_height(term_h, wrapped_lines)
            .try_into()
            .unwrap_or(u16::MAX)
    }

    /// The wrapped result at the current width: keys and rendering share it.
    fn wrapped(&self) -> crate::ui::tui::text::Wrapped {
        crate::ui::tui::text::wrap(&self.editor.text(), geometry::inner_width(self.term.cols))
    }

    /// Apply one intent: editor actions drive the editor, the rest the history ring.
    /// `true` = consumed.
    fn apply(
        &mut self,
        intent: &semantics::Intent,
        wrapped: Option<&crate::ui::tui::text::Wrapped>,
    ) -> bool {
        use semantics::Intent as I;
        let mut self_managed = false;
        let eff = match intent {
            I::Insert(c) => self.editor.insert_char(*c),
            I::Paste(s) => {
                let before = self.editor.len();
                let _ = self.editor.insert_paste(s);
                if self.editor.len() != before {
                    Effect::Content
                } else {
                    Effect::Nothing
                }
            }
            I::Newline => self.editor.insert_str("\n"),
            I::Backspace => self.editor.backspace(),
            I::Delete => self.editor.delete(),
            I::Left => self.editor.left(),
            I::Right => self.editor.right(),
            I::Up if !self.editor_is_multiline() || self.history.browsing() => {
                if self.finished || self.history.browsing() {
                    if let Some(text) = self.history.previous(&self.editor.text()) {
                        self.editor = Editor::from_text(&text);
                    }
                    return true;
                }
                Effect::Nothing
            }
            I::Up => self.editor.up(wrapped_expect(wrapped), &mut self.goal_col),
            I::Down if self.history.browsing() => {
                if let Some(text) = self.history.next_entry() {
                    self.editor = Editor::from_text(&text);
                }
                return true;
            }
            I::Down => self
                .editor
                .down(wrapped_expect(wrapped), &mut self.goal_col),
            I::LineHome => self.editor.line_home(),
            I::LineEnd => self.editor.line_end(),
            I::DocHome => self.editor.home(),
            I::DocEnd => self.editor.end(),
            I::WordLeft => self.editor.word_left(),
            I::WordRight => self.editor.word_right(),
            I::DeleteWordBackward => self.editor.delete_word_backward(),
            I::DeleteWordForward => self.editor.delete_word_forward(),
            I::DeleteToLineStart => self.editor.delete_to_line_start(),
            I::DeleteToLineEnd => self.editor.delete_to_line_end(),
            I::Undo => {
                self.editor.undo();
                Effect::Content
            }
            I::Redo => {
                self.editor.redo();
                Effect::Content
            }
            I::Submit => {
                // What leaves is the **expanded** text: folded pastes live in the
                // editor's store, which `clear()` below drops.
                let text = self.editor.expanded_text();
                let route = route::classify(&text);
                self.data.hint = None;
                // A live round with a message in the box: the first Enter interrupts, a second
                // one inside the window sends it to the slot. A command never interrupts.
                if self.data.busy
                    && !text.trim().is_empty()
                    && matches!(route, route::Route::Message)
                {
                    let now = std::time::Instant::now();
                    let again = self
                        .enter_armed
                        .take()
                        .is_some_and(|t| now.duration_since(t) < ESC_TREE_WINDOW);
                    if !again {
                        self.enter_armed = Some(now);
                        self.pending_exits.push(ExitRequest::Interrupt);
                        return true;
                    }
                }
                if text.trim().is_empty() {
                    Effect::Nothing
                } else if let route::Route::Command(meta, args) = route {
                    let call = command::Call {
                        args,
                        attached: self.data.attached,
                    };
                    match command::run(meta, &call) {
                        command::Done::Intent(request) => {
                            self.note_intent(&request);
                            self.outbox.push(request);
                        }
                        command::Done::Exit(exit) => self.pending_exits.push(exit),
                        command::Done::Hire(who) => self.pending_hire = Some(who),
                        command::Done::Profile(profile) => {
                            self.outbox.push(Request::PickProfile(profile));
                        }
                        command::Done::Hint(line) => self.data.hint = Some(line.to_string()),
                        // A required argument is missing: the line stays where it can be fixed.
                        command::Done::Hold => return true,
                    }
                    self.history.push(self.editor.text());
                    self.editor.clear();
                    self.finished = true;
                    self_managed = true;
                    Effect::Content
                } else if self.data.busy {
                    // A round is running: the sentence waits in the slot, in order.
                    self.history.push(self.editor.text());
                    self.editor.clear();
                    self.finished = true;
                    self_managed = true;
                    if let Some(request) = self.queue.push(Outgoing::new(text)) {
                        self.outbox.push(request);
                    }
                    self.shouts.push(Shout::new(
                        name::QUEUE,
                        serde_json::json!({"count": self.queue.len()}),
                    ));
                    Effect::Content
                } else {
                    self.route_out(&text);
                    self.in_flight = Some(text.clone());
                    self.shouts.push(Shout::new(
                        name::ECHO,
                        serde_json::json!({"text": text, "prefix": null, "suffix": null}),
                    ));
                    self.history.push(self.editor.text());
                    self.editor.clear();
                    self.finished = true;
                    self_managed = true;
                    self.data.busy = true;
                    self.shouts
                        .push(Shout::new(name::BUSY, serde_json::json!({"on": true})));
                    Effect::Content
                }
            }
            I::TakeBack => {
                // The newest queued sentence comes back above whatever is already in the box.
                let (taken, request) = self.queue.pop();
                if let Some(request) = request {
                    self.outbox.push(request);
                }
                self.shouts.push(Shout::new(
                    name::QUEUE,
                    serde_json::json!({"count": self.queue.len()}),
                ));
                match taken {
                    Some(msg) => {
                        let sep = if self.editor.is_empty() { "" } else { "\n" };
                        self.editor
                            .replace_range(0, 0, &format!("{}{sep}", msg.text));
                        Effect::Content
                    }
                    None => Effect::Nothing,
                }
            }
            I::ClearInput => {
                self.editor.clear();
                self.history.exit();
                self.finished = true;
                self_managed = true;
                Effect::Content
            }
            I::None => Effect::Nothing,
        };
        match eff {
            Effect::Nothing => false,
            _ => {
                if matches!(eff, Effect::Content) {
                    if !self_managed {
                        self.finished = false;
                    }
                    self.history.on_edit();
                }
                true
            }
        }
    }

    /// A message line leaves as a turn.
    fn route_out(&mut self, text: &str) {
        self.pending_exits
            .push(ExitRequest::Submit(text.to_string()));
    }

    /// Whether the input spans more than one line.
    fn editor_is_multiline(&self) -> bool {
        self.editor.text().contains('\n')
    }

    /// Drop the draft and every transient interaction. Submission history stays.
    pub fn clear(&mut self) {
        self.editor.clear();
        self.history.exit();
        self.finished = true;
        self.scroll = 0;
        self.pending_hire = None;
        self.hang = false;
        self.esc_armed = None;
        self.enter_armed = None;
        self.pending_exits.clear();
        self.queue.clear();
        self.in_flight = None;
        self.last_mailbox_turn = None;
    }

    /// What a rewind put back: the text goes in as it was, cursor at the end. An
    /// existing draft is not overwritten.
    pub fn prefill(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.editor.is_empty() {
            self.editor = Editor::from_text(text);
            self.finished = false;
            self.history.on_edit();
        }
    }

    /// Esc, when nothing borrowed it: it interrupts, and a second tap within the
    /// window also asks for the node tree.
    fn route_esc(&mut self, key: &Key) -> bool {
        if key.mods.alt || key.mods.ctrl {
            return false;
        }
        self.pending_exits.push(ExitRequest::Interrupt);
        let now = std::time::Instant::now();
        match self.esc_armed.take() {
            Some(t) if now.duration_since(t) < ESC_TREE_WINDOW => {
                self.pending_exits.push(ExitRequest::OpenTree);
            }
            _ => self.esc_armed = Some(now),
        }
        true
    }

    /// The replacement a hand computed for this editor. Char indices, like the engine.
    pub fn apply_reply(&mut self, reply: Reply) {
        let Reply::Replace { from, to, text } = reply;
        self.editor.replace_range(from, to, &text);
        self.finished = false;
        self.data.hint = None;
        self.refresh_hang();
        if self.hang {
            self.pending_hire = Some(Hire::Completion);
        }
    }

    /// Whether the cursor sits in a completion trigger word, from the editor's own
    /// text. Triggers: a `/` as the input's first character; or a word preceded by a
    /// space that starts with `./`, `../`, `~/`, a `/` plus one more character, or a
    /// leading path segment followed by `/`.
    fn refresh_hang(&mut self) {
        let text = self.editor.text();
        let cursor = self.editor.cursor();
        let chars: Vec<char> = text.chars().collect();
        let cursor = cursor.min(chars.len());
        // The trigger word is the whole non-space run: `src/main.rs` is one word here, even
        // though word delete stops at the slash.
        let mut start = cursor;
        while start > 0 && !chars[start - 1].is_whitespace() {
            start -= 1;
        }
        let word: String = chars[start..cursor].iter().collect();
        let spaced = start == 0 || chars[start - 1] == ' ';
        let is_command = start == 0 && word.starts_with('/');
        let head = word.split('/').next().unwrap_or("");
        let ascii_head = !head.is_empty()
            && head
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
        let is_path = word.starts_with("./")
            || word.starts_with("../")
            || word.starts_with("~/")
            || (word.starts_with('/') && word.chars().count() > 1)
            || (word.contains('/') && ascii_head);
        let argument = self.command_argument(&chars, cursor, start);
        self.hang = argument || (spaced && !word.is_empty() && (is_command || is_path));
    }

    /// A write the user just asked for: the widgets follow the address it already names.
    fn note_intent(&mut self, request: &Request) {
        if let Request::SwitchModel(id) = request {
            let address = format!("{}:{}", id.provider, id.model);
            let mut out = Vec::new();
            self.model_address(address, &mut out);
            self.outbox.extend(out);
        }
    }

    /// An edit landed: the word decides whether the completion hand is wanted.
    fn edited(&mut self, consumed: bool) -> bool {
        self.refresh_hang();
        if self.hang {
            self.pending_hire = Some(Hire::Completion);
        }
        consumed
    }

    /// Whether the cursor sits in an argument position that has candidates.
    fn command_argument(&self, chars: &[char], cursor: usize, start: usize) -> bool {
        let before: String = chars[..cursor].iter().collect();
        let words: Vec<&str> = before.split_whitespace().collect();
        let ends_space = before.chars().last().is_some_and(char::is_whitespace);
        let index = match ends_space {
            true => words.len(),
            false => words.len().saturating_sub(1),
        };
        if index == 0 || start == 0 {
            return false;
        }
        let Some(meta) = words.first().and_then(|name| command::lookup(name)) else {
            return false;
        };
        matches!(
            meta.arg(index - 1),
            Some(
                crate::ui::command::Arg::Path
                    | crate::ui::command::Arg::Model
                    | crate::ui::command::Arg::Profile
            )
        )
    }

    /// The statusline's own reading of an event.
    fn notify(&mut self, event: &str, content: &Value) {
        let ctx = Ctx { data: &self.data };
        self.statusline.on_event(event, content, &ctx);
    }

    fn ctx(&self) -> Ctx<'_> {
        Ctx { data: &self.data }
    }

    /// One frame answer the zone reads for itself.
    fn read_frame(&mut self, frame: &Frame) -> Vec<Request> {
        let mut out = Vec::new();
        match (frame.class.as_str(), frame.name.as_str()) {
            ("call", "meta") => {
                self.read_meta(&frame.content, &mut out);
            }
            ("stream", "change") => {
                if let Some(row) = frame.content.get("meta") {
                    self.read_meta(row, &mut out);
                }
            }
            ("call", "config") => {
                self.read_config(&frame.content);
            }
            ("post", "cwd") => {
                self.read_cwd(&frame.content);
            }
            ("post", "switch") => {
                if frame.content.get("error").is_some() {
                    out.push(Request::Meta);
                }
            }
            ("call", "usage") => {
                self.read_usage(&frame.content);
            }
            ("subscribe", "submit") => {
                if frame.content.get("error").is_some() {
                    out.extend(self.refused(&frame.content));
                } else {
                    self.in_flight = None;
                }
            }
            ("stream", "mailbox") => {
                out.extend(self.opened(&frame.content));
            }
            _ => {}
        }
        out
    }

    /// The reply to a submit that opened nothing: the sentence comes back, and says why.
    fn refused(&mut self, content: &Value) -> Vec<Request> {
        let reason = content
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("the session is still in a turn")
            .to_string();
        self.shouts.push(Shout::new(
            name::RETRACT,
            serde_json::json!({"reason": reason}),
        ));
        if let Some(text) = self.in_flight.take() {
            self.prefill(&text);
        }
        Vec::new()
    }

    /// The slot became a turn: the head left the queue, and the next one takes the slot.
    fn opened(&mut self, content: &Value) -> Vec<Request> {
        let Some(turn) = content.get("turn").and_then(Value::as_i64) else {
            return Vec::new();
        };
        if self.last_mailbox_turn == Some(turn) {
            return Vec::new();
        }
        self.last_mailbox_turn = Some(turn);
        let (taken, request) = self.queue.took();
        let out: Vec<Request> = request.into_iter().collect();
        self.shouts.push(Shout::new(
            name::QUEUE,
            serde_json::json!({"count": self.queue.len()}),
        ));
        if let Some(msg) = taken {
            self.shouts.push(Shout::new(
                name::ECHO,
                serde_json::json!({"text": msg.text, "prefix": msg.prefix, "suffix": msg.suffix}),
            ));
        }
        out
    }

    /// The session row: name, and the model its widgets follow.
    fn read_meta(&mut self, row: &Value, out: &mut Vec<Request>) {
        if let Some(name) = row.get("name").and_then(Value::as_str) {
            self.data.name = Some(name.to_string());
        }
        let Some(provider) = row.get("provider").and_then(Value::as_str) else {
            return;
        };
        let Some(model) = row.get("model").and_then(Value::as_str) else {
            return;
        };
        let address = format!("{provider}:{model}");
        if self.data.address.as_deref() == Some(address.as_str()) {
            return;
        }
        self.model_address(address, out);
    }

    /// One model's facts, all through the config door: name, window, currency, unit prices.
    fn model_address(&mut self, address: String, out: &mut Vec<Request>) {
        self.data.address = Some(address.clone());
        self.data.model = None;
        self.data.limit = 0;
        self.data.prices = None;
        for field in [
            "name",
            "contextWindow",
            "currency",
            "cost.input",
            "cost.output",
            "cost.cacheRead",
            "cost.cacheWrite",
        ] {
            out.push(Request::Config {
                key: format!("models.model.{address}.{field}"),
            });
        }
    }

    /// One config answer: the statusline's own keys, and the running model's facts.
    fn read_config(&mut self, content: &Value) {
        let Some(key) = content.get("key").and_then(Value::as_str) else {
            return;
        };
        let value = content.get("value");
        if key == DEFAULT_MODEL_KEY {
            if self.data.address.is_none()
                && let Some(address) = value
                    .and_then(Value::as_str)
                    .filter(|address| !address.is_empty())
            {
                let mut out = Vec::new();
                self.model_address(address.to_string(), &mut out);
                self.outbox.extend(out);
            }
            return;
        }
        if let Some(rest) = key.strip_prefix("models.model.") {
            self.read_model_key(rest, value);
            return;
        }
        if key.ends_with(".deco.gitScanInterval")
            && let Some(seconds) = value.and_then(Value::as_u64)
        {
            let interval = std::time::Duration::from_secs(seconds.max(1));
            if interval != self.setup.git_scan {
                self.setup.git_scan = interval;
                self.statusline = StatusLine::new(&self.setup);
                // The fresh decos were built after the workspace arrived: hand it over again.
                if !self.data.cwd.is_empty() {
                    let content = serde_json::json!({"path": self.data.cwd});
                    self.notify(crate::ui::tui::zone::name::CWD, &content);
                }
            }
        }
    }

    /// `<address>.<field>`, the address itself holding no dot.
    fn read_model_key(&mut self, rest: &str, value: Option<&Value>) {
        const FIELDS: [&str; 7] = [
            "cost.cacheWrite",
            "cost.cacheRead",
            "cost.output",
            "cost.input",
            "contextWindow",
            "currency",
            "name",
        ];
        let Some(field) = FIELDS
            .iter()
            .find(|field| rest.ends_with(&format!(".{field}")))
            .copied()
        else {
            return;
        };
        let address = rest.strip_suffix(&format!(".{field}")).unwrap_or_default();
        if self.data.address.as_deref() != Some(address) {
            return;
        }
        let number = value.and_then(Value::as_f64).unwrap_or(0.0);
        match field {
            "name" => {
                self.data.model = value.and_then(Value::as_str).map(str::to_string);
            }
            "contextWindow" => {
                self.data.limit = value.and_then(Value::as_u64).unwrap_or(0);
            }
            "currency" => {
                self.data.money.symbol = match value.and_then(Value::as_str) {
                    Some("CNY") => "¥".to_string(),
                    Some("USD") => "$".to_string(),
                    _ => String::new(),
                };
            }
            other => {
                let prices = self.data.prices.get_or_insert_with(Prices::default);
                match other {
                    "cost.input" => prices.input = number,
                    "cost.output" => prices.output = number,
                    "cost.cacheRead" => prices.cache_read = number,
                    _ => prices.cache_write = number,
                }
            }
        }
    }

    /// The bill: one currency, the model's own. A stored total replaces what was counted.
    fn read_usage(&mut self, content: &Value) {
        let Some(groups) = content.get("groups").and_then(Value::as_array) else {
            return;
        };
        let mut total = 0.0;
        let mut currency = String::new();
        for group in groups {
            let Some(price) = group.get("price") else {
                continue;
            };
            if currency.is_empty()
                && let Some(name) = price.get("currency").and_then(Value::as_str)
            {
                currency = name.to_string();
            }
            total += price.get("total").and_then(Value::as_f64).unwrap_or(0.0);
        }
        self.data.money.base = total;
        self.data.money.live = 0.0;
        if !currency.is_empty() {
            self.data.money.symbol = match currency.as_str() {
                "CNY" => "¥".to_string(),
                "USD" => "$".to_string(),
                _ => self.data.money.symbol.clone(),
            };
        }
    }

    /// The write landed: the workspace is the one the server resolved, and everyone hears it.
    fn read_cwd(&mut self, content: &Value) {
        if content.get("error").is_some() {
            return;
        }
        let Some(path) = content.get("cwd").and_then(Value::as_str) else {
            return;
        };
        self.data.cwd.clear();
        self.data.cwd.push_str(path);
        self.data.temporary = false;
        self.shouts.push(Shout::new(
            name::CWD,
            serde_json::json!({"path": path, "temporary": false}),
        ));
    }
}

/// `Editor::up` / `down` need a `Wrapped`; an empty one stands in when the caller
/// has none (an empty box rewinds history instead).
fn empty_wrapped() -> &'static crate::ui::tui::text::Wrapped {
    static EMPTY: std::sync::LazyLock<crate::ui::tui::text::Wrapped> =
        std::sync::LazyLock::new(|| crate::ui::tui::text::wrap("", 1));
    &EMPTY
}

fn wrapped_expect(w: Option<&crate::ui::tui::text::Wrapped>) -> &crate::ui::tui::text::Wrapped {
    w.unwrap_or_else(|| empty_wrapped())
}

impl SubZone for InputZone {
    fn claims(&self) -> &'static [Claim] {
        Self::CLAIMS
    }

    fn kinds(&self) -> &'static [crate::ui::tui::zone::Kind] {
        &[
            crate::ui::tui::zone::Kind::Call,
            crate::ui::tui::zone::Kind::Post,
            crate::ui::tui::zone::Kind::Stream,
            crate::ui::tui::zone::Kind::Notice,
        ]
    }

    fn deliver_event(&mut self, frame: &Frame, size: TermSize) -> Vec<Request> {
        self.term = size;
        self.read_frame(frame)
    }

    /// Esc first (a lent-away Esc never arrives here), then the intent table.
    fn deliver_action(&mut self, a: &Action) -> bool {
        self.data.hint = None;
        let key = match a {
            Action::Key(key) => *key,
            Action::Paste(text) => {
                let intent = semantics::Intent::Paste(text.clone());
                let consumed = self.apply(&intent, None);
                return self.edited(consumed);
            }
            Action::ScrollUp { .. } | Action::ScrollDown { .. } => return false,
        };
        if key.code == KeyCode::Esc && self.route_esc(&key) {
            return true;
        }
        let intent = semantics::translate(key);
        if matches!(intent, semantics::Intent::None) {
            return false;
        }
        let wrapped = matches!(intent, semantics::Intent::Up | semantics::Intent::Down)
            .then(|| self.wrapped());
        let consumed = self.apply(&intent, wrapped.as_ref());
        self.edited(consumed)
    }

    fn attach(&mut self, _session: i64, _size: TermSize) {
        self.restart_data();
        self.data.attached = true;
        self.queue.clear();
        self.in_flight = None;
        self.last_mailbox_turn = None;
        self.outbox.push(Request::Usage);
    }

    fn detach(&mut self) {
        self.pending_exits.clear();
        self.pending_hire = None;
        self.hang = false;
        self.esc_armed = None;
        self.enter_armed = None;
        self.editor.clear();
        self.history.exit();
        self.finished = true;
        self.scroll = 0;
        self.restart_data();
        self.queue.clear();
        self.in_flight = None;
        self.last_mailbox_turn = None;
        self.outbox.push(Request::Config {
            key: DEFAULT_MODEL_KEY.to_string(),
        });
    }

    fn take_shouts(&mut self) -> Vec<Shout> {
        std::mem::take(&mut self.shouts)
    }

    fn hear(&mut self, event: &str, content: &Value) {
        match event {
            name::CWD => {
                if let Some(path) = content.get("path").and_then(Value::as_str) {
                    self.data.cwd.clear();
                    self.data.cwd.push_str(path);
                    if let Some(temporary) = content.get("temporary").and_then(Value::as_bool) {
                        self.data.temporary = temporary;
                    }
                    self.notify(event, content);
                }
            }
            name::USAGE => {
                if let Some(used) = content.get("used").and_then(Value::as_u64) {
                    self.data.used = used;
                }
            }
            name::BILL => {
                if let Some(prices) = self.data.prices.as_ref() {
                    let bill = Bill {
                        prompt: content.get("prompt").and_then(Value::as_u64).unwrap_or(0),
                        completion: content
                            .get("completion")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                        cached: content.get("cached").and_then(Value::as_u64).unwrap_or(0),
                        cache_write: content
                            .get("cache_write")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                    };
                    self.data.money.live += prices.bill(&bill);
                }
            }
            name::TICK => {
                self.notify(event, content);
            }
            name::PREFILL => {
                if let Some(text) = content.get("text").and_then(Value::as_str) {
                    self.prefill(text);
                }
            }
            name::BUSY => {
                self.data.busy = content.get("on").and_then(Value::as_bool).unwrap_or(false);
                if !self.data.busy {
                    self.enter_armed = None;
                    self.outbox.push(Request::Usage);
                }
                self.notify(event, content);
            }
            _ => {}
        }
    }
}

impl Pane for InputZone {
    fn request_height(&self, term: TermSize) -> u16 {
        self.request_height_for(self.wrapped_for(term).len(), term.rows)
    }

    fn assign(&mut self, term: TermSize, rows: u16) {
        self.term = term;
        self.rows = rows;
    }
}

impl InputZone {
    /// The wrapped result at the given width — `request_height` runs before the new
    /// size is stored, so the width comes in as an argument.
    fn wrapped_for(&self, term: TermSize) -> crate::ui::tui::text::Wrapped {
        crate::ui::tui::text::wrap(&self.editor.text(), geometry::inner_width(term.cols))
    }

    /// Self-rendering; the width comes in as an argument, like its siblings.
    ///
    /// Order: wrap, scroll to the cursor, the statusline row, then the container.
    pub fn render_rows(
        &mut self,
        term: TermSize,
        accent: &Accent,
    ) -> Vec<ratatui::text::Line<'static>> {
        self.term = term;
        let term_w = term.cols.max(1);
        let wrapped =
            crate::ui::tui::text::wrap(&self.editor.text(), geometry::inner_width(term_w));
        let (cursor_row, cursor_col) = wrapped.locate(self.editor.cursor());
        let container_rows = usize::from(self.rows.max(2));
        self.scroll =
            geometry::adjust_scroll(self.scroll, wrapped.len(), container_rows, cursor_row);
        let status_line = {
            let ctx = self.ctx();
            self.statusline
                .render(&StatusTheme::new(*accent), term_w, &ctx)
        };
        let drawn = render::render(
            &render::Spec {
                status_line,
                wrapped: &wrapped,
                starts: self.scroll,
                container_rows,
                cursor_row,
                cursor_col,
                term_w,
            },
            accent,
        );
        self.cursor_col = drawn.cursor_col as u16;
        self.cursor_row = drawn.cursor_row as u16;
        drawn.lines
    }
}
