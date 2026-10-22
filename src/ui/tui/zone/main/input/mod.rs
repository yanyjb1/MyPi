//! The input zone: editor, input history, statusline, submit, command routing.
//! It does not know what sits above it; it only knows when it lent its keys away.

use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::SubZone;
use crate::ui::tui::zone::main::geometry;
use crate::ui::tui::zone::main::reserved::{Hire, Reply};
use crate::ui::tui::zone::{Action, Claim, Ctx, Event, Key, KeyCode, Request, TermSize};

pub mod editor;
pub mod render;
pub mod route;
pub mod semantics;
pub mod statusline;

use editor::{Editor, Effect, History};
use statusline::{Setup, StatusLine, StatusTheme};

/// What the editor declares when it cannot act on its own. Where each one goes
/// is the zone's, the app's and the session's business.
///
/// Two un-lent Esc this close together ask for the node tree.
pub const ESC_TREE_WINDOW: std::time::Duration = std::time::Duration::from_millis(300);

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
    /// Idle and rewinding: the box is empty, so ↑↓ walks the history. Any typing
    /// clears it.
    pub finished: bool,
    /// The cursor sits in a trigger word: the condition for lending keys.
    pub hang: bool,
    /// The hand to hire, produced here and taken by the zone.
    pub pending_hire: Option<Hire>,
    /// The last ↑↓ goal column, carried for `Editor::up` / `down`.
    pub goal_col: Option<usize>,
    /// Exit requests, in order: taken upward one by one (input → main → app → loop).
    /// A queue, not a slot: one batch may carry two Esc.
    pub pending_exits: Vec<ExitRequest>,
    /// When the last un-lent Esc fired: another one inside the window opens the tree.
    esc_armed: Option<std::time::Instant>,
    /// When Enter was last pressed mid-round: another one inside the window submits.
    enter_armed: Option<std::time::Instant>,
    /// The cursor's absolute terminal column, border included; recomputed each
    /// render, used to anchor the completion popup.
    pub cursor_col: u16,
    /// The cursor's row inside the container (0 = the statusline row). The zone
    /// reports container-relative; the main zone converts to terminal coordinates.
    pub cursor_row: u16,
    /// The statusline: the top edge of the container and its own deco table.
    pub statusline: StatusLine,
    /// The terminal size from this round's arbitration; wrapping needs it between
    /// resizes.
    term: crate::ui::tui::zone::TermSize,
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
            esc_armed: None,
            enter_armed: None,
            cursor_col: geometry::BORDER_COLS as u16,
            cursor_row: 1,
            statusline: StatusLine::new(&Setup::default()),
            term: crate::ui::tui::zone::TermSize { cols: 80, rows: 24 },
        }
    }
}

impl InputZone {
    /// Claims: any printable character and pastes.
    pub const CLAIMS: &[Claim] = &[Claim::AnyChar, Claim::Paste];

    /// The height it asks for: content rows, capped at a share of the terminal.
    pub fn request_height_for(&self, wrapped_lines: usize, term_h: u16) -> u16 {
        geometry::container_height(term_h, wrapped_lines)
            .try_into()
            .unwrap_or(u16::MAX)
    }

    /// The wrapped result at the current width: keys and rendering share it.
    fn wrapped(&self) -> crate::ui::tui::text::Wrapped {
        crate::ui::tui::text::wrap(&self.editor.text(), geometry::inner_width(self.term.cols))
    }
}

impl InputZone {
    /// Apply one intent: editor actions drive the editor, the rest the history ring.
    /// `true` = consumed.
    fn apply(
        &mut self,
        intent: &semantics::Intent,
        wrapped: Option<&crate::ui::tui::text::Wrapped>,
        ctx: &Ctx,
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
                // A live round with text in the box: the first Enter interrupts, a second
                // one inside the window submits.
                if ctx.view.busy && !self.editor.is_empty() {
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
                // What leaves is the **expanded** text: folded pastes live in the
                // editor's store, which `clear()` below drops.
                let text = self.editor.expanded_text();
                if text.trim().is_empty() {
                    Effect::Nothing
                } else {
                    self.route_out(&text);
                    self.history.push(self.editor.text());
                    self.editor.clear();
                    self.finished = true;
                    self_managed = true;
                    Effect::Content
                }
            }
            I::Interrupt => {
                self.pending_exits.push(ExitRequest::Interrupt);
                Effect::Nothing
            }
            I::ClearInput => {
                self.editor.clear();
                self.history.exit();
                self.finished = true;
                self_managed = true;
                Effect::Content
            }
            I::ToggleReasoning | I::ToggleTools | I::None => Effect::Nothing,
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

    /// One submitted line: a command the table knows, or a message.
    fn route_out(&mut self, text: &str) {
        let route::Route::Command(meta, args) = route::classify(text) else {
            self.pending_exits
                .push(ExitRequest::Submit(text.to_string()));
            return;
        };
        if self.local_command(meta, args) {
            return;
        }
        if meta.name == crate::commands::Usage::META.name {
            self.pending_hire = Some(Hire::Usage);
        }
        self.pending_exits
            .push(ExitRequest::Submit(text.to_string()));
    }

    /// The commands this front end runs itself. `false` = not one of ours.
    fn local_command(&mut self, meta: &'static crate::commands::CommandMeta, _args: &str) -> bool {
        if meta.name == crate::commands::Quit::META.name {
            self.pending_exits.push(ExitRequest::Quit);
            return true;
        }
        if meta.name == crate::commands::Resume::META.name {
            self.pending_exits.push(ExitRequest::OpenResume);
            return true;
        }
        if meta.name == crate::commands::New::META.name {
            self.pending_exits.push(ExitRequest::New);
            return true;
        }
        false
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
    }

    /// What a rewind put back: the text goes in as it was, cursor at the end. An
    /// existing draft is not overwritten; the new text is appended after it.
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
        self.refresh_hang();
    }

    /// Whether the cursor sits in a completion trigger word, from the editor's own
    /// text. Word bounds are the separator table's hard bounds.
    ///
    /// Triggers: a `/` as the input's first character (the command line); or a word
    /// preceded by a space that starts with `./`, `../`, `~/`, a `/` plus one more
    /// character, or a leading path segment followed by `/`.
    fn refresh_hang(&mut self) {
        let text = self.editor.text();
        let cursor = self.editor.cursor();
        let chars: Vec<char> = text.chars().collect();
        let cursor = cursor.min(chars.len());
        let mut start = cursor;
        while start > 0 && !self.editor.separators.is_hard(chars[start - 1]) {
            start -= 1;
        }
        let word: String = chars[start..cursor].iter().collect();
        // The word must follow a space, or be the start of the text.
        let spaced = start == 0 || chars[start - 1] == ' ';
        let is_command = start == 0 && word.starts_with('/');
        // A bare relative path counts only when its first segment looks like one:
        // `/` inside ordinary prose is not a path.
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
        self.hang = spaced && !word.is_empty() && (is_command || is_path);
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

    /// Events carry no data: what a round is doing is read from `ctx.view`.
    fn deliver_event(&mut self, ev: Event, ctx: &Ctx) {
        self.term = ctx.size;
        if matches!(ev, Event::RoundStarted | Event::RoundEnded) {
            self.enter_armed = None;
        }
        self.statusline.on_event(ev, ctx);
    }

    /// Esc first (a lent-away Esc never arrives here), then the intent table.
    fn deliver_action(&mut self, a: &Action, ctx: &Ctx) -> bool {
        self.term = ctx.size;
        let key = match a {
            Action::Key(key) => *key,
            Action::Paste(text) => {
                let intent = semantics::Intent::Paste(text.clone());
                return self.apply(&intent, None, ctx);
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
        let consumed = self.apply(&intent, wrapped.as_ref(), ctx);
        // Hang is re-read after every edit; a fresh hang is what hires the completion.
        let was = self.hang;
        self.refresh_hang();
        if self.hang && !was {
            self.pending_hire = Some(Hire::Completion);
        }
        consumed
    }

    fn attach(&mut self, _session: i64, _ctx: &Ctx) {}

    fn detach(&mut self, _ctx: &Ctx) {
        self.pending_exits.clear();
        self.pending_hire = None;
        self.hang = false;
        self.esc_armed = None;
        self.enter_armed = None;
        self.editor.clear();
        self.history.exit();
        self.finished = true;
        self.scroll = 0;
    }

    /// Only session actions leave here; screen changes stay for Main's dispatch.
    fn take_requests(&mut self) -> Vec<Request> {
        let mut out = Vec::new();
        let mut rest = Vec::new();
        for exit in self.pending_exits.drain(..) {
            match exit {
                ExitRequest::Submit(text) => out.push(Request::Submit(text)),
                ExitRequest::Interrupt => out.push(Request::Interrupt),
                other => rest.push(other),
            }
        }
        self.pending_exits = rest;
        out
    }

    /// Rows wanted: the wrapped content, capped at a fifth of the terminal.
    fn request_height(&self, term: TermSize) -> u16 {
        self.request_height_for(self.wrapped_for(term).len(), term.rows)
    }

    fn assign(&mut self, term: TermSize, rows: u16) {
        self.term = term;
        self.rows = rows;
    }

    fn on_height_changed(&mut self, _rows: u16) -> bool {
        false
    }
}

impl InputZone {
    /// The wrapped result at the given width — `request_height` runs before the new
    /// size is stored, so the width comes in as an argument.
    fn wrapped_for(&self, term: crate::ui::tui::zone::TermSize) -> crate::ui::tui::text::Wrapped {
        crate::ui::tui::text::wrap(&self.editor.text(), geometry::inner_width(term.cols))
    }

    /// Self-rendering; the width comes in as an argument, like its siblings.
    ///
    /// Order: wrap, scroll to the cursor, the statusline row, then the container.
    /// The cursor column is reported to the completion popup along the way.
    pub fn render_rows(
        &mut self,
        term_w: u16,
        p: &crate::ui::tui::theme::Palette,
        view: &crate::ui::session::View<'_>,
    ) -> Vec<ratatui::text::Line<'static>> {
        let term_w = term_w.max(1);
        let wrapped =
            crate::ui::tui::text::wrap(&self.editor.text(), geometry::inner_width(term_w));
        let (cursor_row, cursor_col) = wrapped.locate(self.editor.cursor());
        let container_rows = usize::from(self.rows.max(2));
        self.scroll =
            geometry::adjust_scroll(self.scroll, wrapped.len(), container_rows, cursor_row);
        let ctx = Ctx {
            view,
            size: self.term,
            memo: None,
        };
        let status_line = self
            .statusline
            .render(&StatusTheme::resolve(), term_w, &ctx);
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
            p,
        );
        self.cursor_col = drawn.cursor_col as u16;
        self.cursor_row = drawn.cursor_row as u16;
        drawn.lines
    }
}
