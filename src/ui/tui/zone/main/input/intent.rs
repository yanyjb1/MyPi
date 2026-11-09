//! One intent, applied: the editor, the history ring, and what an edit settles.

use super::super::desk::{Hire, Reply};
use super::command;
use super::editor::{Editor, Effect};
use super::semantics;
use super::zone::InputZone;
use crate::ui::tui::text::Wrapped;
use crate::ui::tui::zone::{Shout, name};

impl InputZone {
    /// Apply one intent: editor actions drive the editor, the rest the history ring.
    /// `true` = consumed.
    pub(super) fn apply(&mut self, intent: &semantics::Intent, wrapped: Option<&Wrapped>) -> bool {
        use semantics::Intent as I;
        let mut self_managed = false;
        let eff = match intent {
            I::Submit => return self.enter(),
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

    /// Whether the input spans more than one line.
    fn editor_is_multiline(&self) -> bool {
        self.editor.text().contains('\n')
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

    /// The replacement a hand computed for this editor. Char indices, like the engine.
    pub fn apply_reply(&mut self, reply: Reply) {
        let Reply::Replace { from, to, text } = reply;
        self.editor.replace_range(from, to, &text);
        self.finished = false;
        self.edited(true);
    }

    /// Whether the cursor sits in a completion trigger: the one rule lives in `command::wants`.
    fn refresh_hang(&mut self) {
        let text = self.editor.text();
        let cursor = self.editor.cursor();
        let before: String = text.chars().take(cursor).collect();
        let (_, word) = crate::ui::command::word(&text, cursor);
        self.hang = command::wants(&before, &word);
    }

    /// An edit landed: the word decides whether the completion hand is wanted.
    pub(super) fn edited(&mut self, consumed: bool) -> bool {
        self.refresh_hang();
        if self.hang {
            self.pending_hire = Some(Hire::Completion);
        }
        consumed
    }
}

/// `Editor::up` / `down` need a `Wrapped`; an empty one stands in when the caller
/// has none (an empty box rewinds history instead).
fn empty_wrapped() -> &'static Wrapped {
    static EMPTY: std::sync::LazyLock<Wrapped> =
        std::sync::LazyLock::new(|| crate::ui::tui::text::wrap("", 1));
    &EMPTY
}

fn wrapped_expect(w: Option<&Wrapped>) -> &Wrapped {
    w.unwrap_or_else(|| empty_wrapped())
}
