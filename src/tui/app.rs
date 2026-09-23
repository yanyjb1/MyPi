//! TUI entry point — event loop and state assembly.
//!
//! Architecture (aligned with pi's two-layer split):
//!
//! ```text
//!   app.rs        state + event loop (this file)
//!     ├── keys.rs       key -> semantic action
//!     ├── editor.rs     editor state (text + cursor)
//!     ├── view.rs       render orchestration
//!     ├── layout.rs     size computation
//!     ├── components/   drawing for each UI region
//!     ├── text.rs       display width and wrapping
//!     └── theme.rs      palette
//! ```
//!
//! Threading model:
//!   main thread: event loop (keyboard + terminal events + background
//!   messages) -> render
//!   background thread: calls client.stream(), emitting a
//!   `SessionEvent` per delta
//!
//! The two threads communicate over `mpsc::channel` (std) — data races
//! are ruled out by the compiler.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyboardEnhancementFlags, KeyEventKind, MouseEventKind,
};
use ratatui::crossterm::execute;

use crate::ai::client::Client;
use crate::ai::config::Config;
use crate::ai::pricing::CostTracker;
use crate::ai::types::{Context as ChatContext, Message};
use crate::entry as entry;
use crate::tui::editor::{Editor, Effect, History};

use crate::tui::keys::{Action, KeyContext, translate_with};
use crate::tui::zones::Zone as _;
use crate::tui::completion::engine;
use crate::tui::layout as tlayout;
use crate::tui::text;
use crate::tui::theme::Palette;
use crate::tui::view::{self, ViewState};

// Spinner frames: `|` `/` `-` `\` cycling. While a turn is in flight
// the loop wakes once per SPIN_INTERVAL to advance it (thinking phases
// produce NO deltas — the spinner must keep spinning on its own). When
// idle the loop sleeps with no timeout at all: zero wake-ups.
const SPINNER: [char; 4] = ['|', '/', '-', '\\'];
const SPIN_INTERVAL: Duration = Duration::from_millis(120);
// Git refresh *rate limit* (a subprocess spawn is expensive; signals can
// burst). Only consulted right after an actual signal — not a poll.
const GIT_REFRESH: Duration = Duration::from_secs(2);

// `editor_action` return value: separates the "language action" from the epilogue.
//
// Why this intermediate layer exists: `apply` needs to distinguish four
// epilogues (nothing / unified finish / load whole text / submit), while
// `editor_action` only applies the key to the editor.
enum EditAction {
    // No-op: touch no state.
    None,
    // Editor mutated; finish with the unified epilogue for that effect.
    Finish(Effect),
    // Load a whole text (history recall); **not** a user edit.
    //
    // The key difference: this must not go through `Effect::Content`,
    // whose epilogue calls `input_history.on_edit()` and clears the
    // browsing state — every ↑ would restart from the newest entry and
    // the same one would come up forever.
    LoadText(String),
    // Submit; hand off to the application layer.
    Submit,
}

// All mutable application state.
pub(crate) struct App {
    pub(crate) editor: Editor,
    // Remembers the target column across consecutive ↑↓ moves, so a
    // short line does not clamp the column and trap the cursor.
    pub(crate) goal_col: Option<usize>,
    // Session service facade: owns turn resources (client/chat/cwd/
    // interrupt) + the SessionState. The TUI only calls protocol methods.
    pub(crate) session: crate::server::Session,
    // Input history (what ↑ cycles through).
    pub(crate) input_history: History,
    // Completion surface (popup state machine + word memo + model args).
    pub(crate) completion: crate::tui::completion::CompletionController,
    pub(crate) tracker: CostTracker,
    // History-zone state (scroll follow, folds). Owned by the zone; the
    // app reads through it when rendering.
    pub(crate) history: crate::tui::zones_impl::HistoryState,
    // Config handle: command argument completion reads the model list.
    pub(crate) cfg: Option<std::rc::Rc<std::cell::RefCell<crate::ai::config::Config>>>,
    // The session's current model entry (statusline display name and /switch follow it).
    pub(crate) current_model: std::rc::Rc<std::cell::RefCell<crate::ai::config::ModelEntry>>,
    // Set by /exit /quit /q: the main loop exits after the current apply finishes.
    pub(crate) quit_requested: bool,
    // /resume picker: Some((candidates, highlighted index)). While Some, the reserved area shows the list.
    pub(crate) resume_pick: Option<(Vec<(i64, String)>, usize)>,
    // Tree navigator modal: Some = full-screen takeover (double-Esc opens it).
    pub(crate) tree_pick: Option<crate::tui::components::tree_picker::TreePicker>,
    // Last Esc press instant, for double-Esc detection.
    pub(crate) last_esc: std::time::Instant,
    // cwd note injected into the first turn after resume (written on resume, consumed on submit).
    pub(crate) pending_cwd_note: Option<String>,
    pub(crate) spin_i: usize,
    pub(crate) git: Option<crate::git::GitStatus>,
    pub(crate) git_at: Instant,
    // The cwd the last git snapshot was taken at. A mismatch after any
    // signal triggers an env refresh — event-driven, zero polling.
    pub(crate) env_cwd: std::path::PathBuf,
    pub(crate) cwd: std::path::PathBuf,
    // Home directory, for `~` expansion in path completion.
    pub(crate) home: std::path::PathBuf,
    // Input viewport start (wrapped row). Independent of the cursor — see `layout::adjust_scroll`.
    pub(crate) scroll: usize,
    // Rendered-block cache: bounded memory, per-frame work bounded to the
    // visible blocks. Survives frames; invalidated inside on width/roster change.
    pub(crate) block_cache: crate::tui::transcript::cache::BlockCache,
}

impl App {
    fn new(
        session: crate::server::Session,
        current_model: std::rc::Rc<std::cell::RefCell<crate::ai::config::ModelEntry>>,
    ) -> Self {
        let cwd = session.cwd();
        let home = std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| cwd.clone());
        Self {
            block_cache: crate::tui::transcript::cache::BlockCache::new(),
            editor: Editor::new(),
            goal_col: None,
            history: crate::tui::zones_impl::HistoryState::default(),
            input_history: History::new(),
            completion: crate::tui::completion::CompletionController::new(),
            tracker: CostTracker::default(),
            spin_i: 0,
            git: crate::git::snapshot(&cwd),
            git_at: Instant::now(),
            env_cwd: cwd.clone(),
            cfg: None,
            quit_requested: false,
            resume_pick: None,
            tree_pick: None,
            last_esc: std::time::Instant::now() - std::time::Duration::from_secs(1),
            pending_cwd_note: None,
            session,
            current_model,
            cwd,
            home,
            scroll: 0,
        }
    }

    // Input wrap width: terminal width minus borders (single definition in `layout::inner_width`).
    fn inner_w(term_w: u16) -> usize {
        tlayout::inner_width(term_w)
    }

    // The current input's wrap result.
    fn wrapped(&self, term_w: u16) -> text::Wrapped {
        text::wrap(&self.editor.text(), Self::inner_w(term_w))
    }

    // Assemble the key-translation context (Esc/Ctrl+C/arrows/Tab all depend on it).
    fn key_context(&self, term_w: u16) -> KeyContext {
        let w = self.wrapped(term_w);
        let (row, _) = w.locate(self.editor.cursor());
        let last = w.len().saturating_sub(1);
        KeyContext {
            editor_empty: self.editor.is_empty(),
            streaming: self.session.busy(),
            popup_open: self.completion.is_open(),
            selector_open: self.resume_pick.is_some(),
            tree_open: self.tree_pick.is_some(),
            at_first_line: row == 0,
            at_last_line: row >= last,
            browsing_history: self.input_history.browsing(),
        }
    }

    // Argument candidates after a complete command (`/model `,
    // `/switch ` -> model id list).
    //
    // The word is "command + trailing content" (e.g. `/model gl`); the
    // filter applies to the **argument part**, and the replacement keeps
    // the command name, filling in only the argument.
    // Command argument candidates, dispatched on the command table's
    // `ArgKind`. `None` = the command has no argument candidates
    // (unknown commands included); `Some(empty)` = it does but nothing
    // matches right now.
    // Reserved-area rows wanted this frame (a layout input).
    //
    // Completion popup open -> candidate rows (capped at MAX); idle ->
    // one blank row. Commands may declare a larger cap; everything
    // currently uses the default 6.
    fn reserved_height(&self, term_h: u16) -> u16 {
        if self.resume_pick.is_some() {
            // Session picker: stretch to 2/3 of the terminal (min 6 rows); the history area shrinks to make room
            let cap = (term_h as usize * 2 / 3).max(6) as u16;
            let want = self.resume_pick.as_ref().map(|(v, _)| v.len() as u16 + 1).unwrap_or(1);
            want.min(cap).min(term_h)
        } else if let Some(h) = self.completion.reserved_height() {
            // Row height locks when the popup opens (inside open()) and
            // stays fixed through filtering — no more per-row jitter.
            h.min(crate::tui::components::reserved::DEFAULT_MAX)
                .min(term_h as usize) as u16
        } else {
            crate::tui::layout::RESERVED_IDLE
        }
    }

    // The path-like word before the cursor (for completion).
    // Refresh completion candidates (called after input changes).
    //
    // No path-like word before the cursor -> close the popup. This is why the popup vanishes when the cursor moves.
    // Tab: advance the completion.
    //
    // Semantics match bash and mainstream editors:
    // 1. popup closed -> open. A single candidate is applied at once;
    //    multiple candidates are only listed, text untouched.
    // 2. popup open -> try extending the common prefix first; if that
    //    cannot advance, confirm the highlighted entry.
    //
    // Why the first candidate is **not** auto-applied on multiple
    // matches: that would silently rewrite a path the user only wanted
    // to look at. Listing candidates and letting ↑↓ choose is
    // predictable.
    // Open the tree navigator (double-Esc). Builds the full-tree rows from
    // the store; a missing store degrades to an empty picker (never crashes).
    fn open_tree_picker(&mut self) {
        let Some(st) = self.session.store() else { return };
        let Some(sid) = self.session.session_id() else {
            self.session.echo(entry::Entry::Error { text: "还没有会话可回溯".into() });
            return;
        };
        let tree = st.load_tree(sid).unwrap_or_default();
        let leaf = st.get_leaf(sid).unwrap_or(None);
        if tree.is_empty() {
            self.session.echo(entry::Entry::Error { text: "会话为空".into() });
            return;
        }
        self.tree_pick = Some(crate::tui::components::tree_picker::TreePicker::from_tree(&tree, leaf));
    }

    pub(crate) fn load_into_editor(&mut self, text: &str) {
        self.editor.clear();
        self.editor.insert_paste(text);
        self.goal_col = None;
        self.completion.close();
    }

    // Apply a completion action to the editor (replace the word segment).
    fn apply_completion(&mut self, action: engine::CompletionAction) {
        if let engine::CompletionAction::Replace { from, to, text } = action {
            self.editor.replace_range(from, to, &text);
            self.goal_col = None;
            // Rescan right after applying: "/model"'s insert ends with a
            // space, and the next tier (model id candidates) needs this
            // rescan to appear — one Tab/Enter should already show the
            // id list, not require another keystroke.
            self.refresh_completions();
        }
    }

    // ---- completion wiring (thin: assemble the input context) ----

    fn refresh_completions(&mut self) {
        let models = crate::tui::completion::controller::models_from_config(&self.cfg);
        let text = self.editor.text().to_string();
        let cx = crate::tui::completion::InputCtx {
            text: &text,
            cursor: self.editor.cursor(),
            cwd: &self.cwd,
            home: &self.home,
            models,
        };
        self.completion.refresh(&cx);
    }

    // Tab: advance the completion. See `CompletionController::on_tab` for
    // the state machine; this wrapper re-refreshes after applying so the
    // argument tier ("/model " -> id list) opens immediately.
    fn complete(&mut self) {
        if !self.completion.is_open() {
            self.refresh_completions();
            if !self.completion.is_open() {
                return; // no candidates
            }
            // Single candidate: apply directly, saving a keystroke
            if self.completion.popup().items().len() == 1 {
                let action = self.completion.accept();
                self.apply_completion(action);
                // When the completion ends with a space ("/model "), the
                // next tier is **argument** candidates; rescan immediately
                // or the argument popup waits for another keystroke.
                self.refresh_completions();
            }
            return; // multiple candidates: popup is open, waiting for a choice
        }

        // Popup open: try the common prefix first (only meaningful with
        // multiple candidates). In argument mode the word ends in a
        // space; use the whole line-start text instead, replacing from 0.
        let current = self
            .editor
            .text()
            .split('\n')
            .next()
            .map(|l| {
                let byte_end = self.editor.cursor().min(l.len());
                l[..byte_end].to_string()
            })
            .unwrap_or_default();
        if let Some(action) = self.completion.accept_common_prefix(&current) {
            self.apply_completion(action);
            self.refresh_completions(); // re-list candidates for the new input
            return;
        }
        // Cannot extend further -> confirm the highlighted entry
        let action = self.completion.accept();
        self.apply_completion(action);
    }

    // ---- end completion wiring ----

    // Execute one semantic action. Returns false to exit.
    //
    // Structure: every "editor-only" action is translated into a
    // language action inside `editor_action`, and the epilogue runs
    // **once, here**. See the `editor_action` docs.
    fn apply(&mut self, action: Action, term_w: u16) -> bool {
        // ---- routing: modal > input (editor) > app-level. One chain,
        // one place. A new overlay only adds a `modal.is_some()` branch
        // here; zones declare their own acceptance in `zones_impl`.
        if self.tree_pick.is_some() {
            match action {
                Action::TreeUp => {
                    if let Some(t) = self.tree_pick.as_mut() {
                        t.move_selection(-1);
                    }
                    return true;
                }
                Action::TreeDown => {
                    if let Some(t) = self.tree_pick.as_mut() {
                        t.move_selection(1);
                    }
                    return true;
                }
                Action::TreeConfirm => {
                    let target = self.tree_pick.as_ref().and_then(|t| t.confirm());
                    self.tree_pick = None;
                    if let Some(seq) = target {
                        self.tree_navigate_to(seq);
                    }
                    return true;
                }
                Action::TreeCancel => {
                    self.tree_pick = None;
                    return true;
                }
                _ => {}
            }
        }
        if self.resume_pick.is_some() {
            match action {
                Action::SelectorUp | Action::SelectorDown => {
                    if let Some((v, sel)) = self.resume_pick.as_mut() {
                        let len = v.len().saturating_sub(1);
                        *sel = match action {
                            Action::SelectorUp => sel.saturating_sub(1),
                            _ => (*sel + 1).min(len),
                        };
                    }
                    return true;
                }
                Action::SelectorConfirm => {
                    self.resume_confirm();
                    return true;
                }
                Action::SelectorCancel => {
                    self.resume_pick = None;
                    return true;
                }
                _ => {}
            }
        }
        // `Action` contains `String`s (paste/insert) so it is not Copy;
        // ownership is only taken when the editor route is actually
        // taken, hence the borrow-based test first.
        if Self::editor_handles(&action) {
            let edit = self
                .editor_action(action, term_w)
                .expect("editor_handles 与 editor_action 的判断必须一致");
            match edit {
                EditAction::None => {}
                EditAction::Finish(effect) => self.finish_edit(effect),
                EditAction::LoadText(text) => self.load_into_editor(&text),
                EditAction::Submit => self.submit(),
            }
            return !self.quit_requested;
        }

        // Everything else is an application-level action: background
        // thread, popup, quitting. Popup actions belong to the input
        // zone (the popup is the input's completion surface); fold and
        // wheel toggles belong to the history zone — each zone's
        // acceptance table lives in `zones_impl`.
        match action {
            Action::EscIdle => {
                // Double-Esc (500ms window, pi's semantics): the first
                // press only arms (stays alive); the second inside the
                // window opens the tree navigator. Pressing nothing
                // further quits — the timeout path `return false` here
                // fires on the **next** idle Esc, not the first one.
                let now = std::time::Instant::now();
                if now.duration_since(self.last_esc).as_millis() < 500 {
                    self.last_esc = now - std::time::Duration::from_secs(1);
                    self.open_tree_picker();
                    return true;
                }
                // First press: arm and keep running. Quit happens via the
                // explicit timeout check below (no Esc queued yet).
                self.last_esc = now;
                return true;
            }
            Action::Quit => return false,

            Action::Complete => self.complete(),
            Action::CompleteUp => self.completion.move_selection(-1),
            Action::CompleteDown => self.completion.move_selection(1),
            Action::DismissCompletion => self.completion.close(),

            Action::ClearInput => self.clear_input(),

            Action::Interrupt => {
                // Only sets the flag: the background thread stops
                // reading and disconnects on the next delta. Killing the
                // thread outright would lose received content and the
                // usage block with it.
                self.session.interrupt_turn();
            }
            Action::ToggleReasoning => {
                // History-zone fold; the next frame recomputes heights.
                self.history.handle(Action::ToggleReasoning);
            }
            Action::ToggleTools => {
                self.history.handle(Action::ToggleTools);
            }

            // Unrecognized key (the default variant): legitimately ignored.
            Action::None => {}

            // Handled by `editor_action`, or semantically a no-op.
            // Reaching here means the two match arms are out of sync —
            // a programming error.
            other => debug_assert!(false, "unhandled action: {other:?}"),
        }
        true
    }

    // Whether `editor_action` handles this action.
    //
    // Exists because `Action` holds `String`s and is not `Copy`, so we
    // cannot try-then-consume. Rather than making `Action` `Copy`
    // (cloning every string), an explicit list is cheaper.
    //
    // **The list must stay in lockstep with `editor_action`'s match
    // arms** — out of sync, the `expect` in `apply` fires immediately,
    // a test run exposes it, and it never becomes silent misbehavior.
    fn editor_handles(action: &Action) -> bool {
        matches!(
            action,
            Action::Insert(_)
                | Action::Paste(_)
                | Action::Newline
                | Action::Backspace
                | Action::Delete
                | Action::DeleteWordBackward
                | Action::DeleteWordForward
                | Action::DeleteToLineStart
                | Action::DeleteToLineEnd
                | Action::Undo
                | Action::Redo
                | Action::Left
                | Action::Right
                | Action::Up
                | Action::Down
                | Action::WordLeft
                | Action::WordRight
                | Action::LineHome
                | Action::LineEnd
                | Action::DocHome
                | Action::DocEnd
                | Action::HistoryPrev
                | Action::HistoryNext
                | Action::Submit
        )
    }

    // Editor actions: translate a key into "which editor method to
    // call".
    //
    // This only applies the language action to the editor and returns
    // it for `apply`; shared epilogues (clear goal column / refresh
    // completions / leave history mode) are deliberately not here.
    //
    // They used to be a 33-arm match inside `apply` with the epilogue
    // hand-copied per arm — `refresh_completions()` 16 times,
    // `reset_goal_col` 8. Missing one line on a new action compiled
    // fine and silently misbehaved; hence the single choke point.
    fn editor_action(&mut self, action: Action, term_w: u16) -> Option<EditAction> {
        let e = match action {
            // ---- input ----
            Action::Insert(c) => self.editor.insert_char(c),
            // The empty string is the Ctrl+V fallback: without bracketed paste the terminal delivers nothing.
            // Intercept early to avoid pushing a pointless undo snapshot.
            Action::Paste(s) => {
                if s.is_empty() {
                    Effect::Nothing
                } else {
                    // Large content folds into a `[paste #N +30 lines]` marker, deletable as a unit.
                    self.editor.insert_paste(&s);
                    Effect::Content
                }
            }
            Action::Newline => self.editor.insert_char('\n'),

            // ---- deletion ----
            Action::Backspace => self.editor.backspace(),
            Action::Delete => self.editor.delete(),
            Action::DeleteWordBackward => self.editor.delete_word_backward(),
            Action::DeleteWordForward => self.editor.delete_word_forward(),
            Action::DeleteToLineStart => self.editor.delete_to_line_start(),
            Action::DeleteToLineEnd => self.editor.delete_to_line_end(),

            // ---- undo ----
            Action::Undo => {
                return Some(if self.editor.undo() {
                    EditAction::Finish(Effect::Content)
                } else {
                    EditAction::None
                });
            }
            Action::Redo => {
                return Some(if self.editor.redo() {
                    EditAction::Finish(Effect::Content)
                } else {
                    EditAction::None
                });
            }

            // ---- motion ----
            Action::Left => self.editor.left(),
            Action::Right => self.editor.right(),
            // ↑↓ target-column memory is maintained by
            // `editor.up/down` themselves; return VerticalMotion so the
            // epilogue does **not** clear goal_col.
            Action::Up => {
                let w = self.wrapped(term_w);
                self.editor.up(&w, &mut self.goal_col);
                return Some(EditAction::Finish(Effect::VerticalMotion));
            }
            Action::Down => {
                let w = self.wrapped(term_w);
                self.editor.down(&w, &mut self.goal_col);
                return Some(EditAction::Finish(Effect::VerticalMotion));
            }
            Action::WordLeft => self.editor.word_left(),
            Action::WordRight => self.editor.word_right(),
            Action::LineHome => self.editor.line_home(),
            Action::LineEnd => self.editor.line_end(),
            Action::DocHome => self.editor.home(),
            Action::DocEnd => self.editor.end(),

            // ---- input history (loads whole text; epilogue is ReplaceAll) ----
            Action::HistoryPrev => {
                let cur = self.editor.text();
                return match self.input_history.previous(&cur) {
                    Some(text) => Some(EditAction::LoadText(text)),
                    None => Some(EditAction::None),
                };
            }
            Action::HistoryNext => {
                return match self.input_history.next_entry() {
                    Some(text) => Some(EditAction::LoadText(text)),
                    None => Some(EditAction::None),
                };
            }

            // ---- submit ----
            Action::Submit => return Some(EditAction::Submit),

            // ---- not this route's business ----
            _ => return None,
        };
        Some(EditAction::Finish(e))
    }

    // The **single** epilogue entry after an editor mutation.
    //
    // What runs is decided entirely by `Effect`, not by individual match arms.
    fn finish_edit(&mut self, effect: Effect) {
        match effect {
            // No-op (cursor already at a boundary, nothing deleted):
            // no goal-column reset, no rescan needed.
            Effect::Nothing => {}
            Effect::Motion => {
                self.goal_col = None;
                self.refresh_completions();
            }
            Effect::VerticalMotion => {
                // Deliberately does not clear `goal_col`: that is
                // exactly the ↑↓ target-column memory.
                self.refresh_completions();
            }
            Effect::Content => {
                self.input_history.on_edit();
                self.goal_col = None;
                self.refresh_completions();
            }
        }
    }

    fn submit(&mut self) {
        // The model receives the **expanded** text: markers are only the on-screen folded view.
        let text = self.editor.expanded_text().trim().to_string();
        if text.is_empty() {
            return;
        }
        // Table-driven dispatch: only complete command names from the
        // table ("/cdp", "/cdp /tmp") count. A prefix ("/cd") is not a
        // command — it goes to the model as prose.
        let (cmd_name, cmd_arg) = match text.split_once(' ') {
            Some((c, a)) => (c, a.trim()),
            None => (text.as_str(), ""),
        };
        if crate::tui::completion::engine::lookup(cmd_name).is_some() {
            self.editor.clear();
            self.completion.close();
            self.goal_col = None;
            self.scroll = 0;
            self.run_command(cmd_name, cmd_arg);
            return;
        }
        if self.session.busy() {
            return;
        }
        self.editor.clear();
        self.input_history.push(text.clone()); // the expanded text
        self.completion.close();
        self.goal_col = None;
        self.scroll = 0;
        self.history.scroll_pinned = true;
        self.history.chat_scroll = 0;
        // First turn after resume: **append** the restored working
        // directory to the user message (history untouched, prefix cache
        // unaffected; older messages stay verbatim).
        let text = match self.pending_cwd_note.take() {
            Some(note) => format!("{text}\n\n{note}"),
            None => text,
        };
        // One call: echo + stage + streaming reset + runner spawn.
        self.session.submit(&text);
    }

    // Clear the input box and reset related state (Ctrl+C).
    fn clear_input(&mut self) {
        self.editor.clear();
        self.input_history.exit();
        self.goal_col = None;
        self.scroll = 0;
        self.completion.close();
    }

    // Environment checkpoint: a turn ended or the cwd moved — the two
    // natural moments the git snapshot can be wrong. Event-driven (the
    // caller checks after a signal), never polled; the rate limit only
    // absorbs bursts of signals.
    fn refresh_env(&mut self) {
        if self.git_at.elapsed() < GIT_REFRESH {
            return;
        }
        self.cwd = self.session.cwd();
        self.env_cwd = self.cwd.clone();
        self.git = crate::git::snapshot(&self.cwd);
        self.git_at = Instant::now();
    }

    // Advance the spinner one frame. Called once per loop iteration;
    // during a turn the loop's wake cadence IS the spinner cadence.
    fn advance_spinner(&mut self) {
        if self.session.busy() {
            self.spin_i = (self.spin_i + 1) % SPINNER.len();
        }
    }

    fn spinner(&self) -> Option<char> {
        self.session.busy().then(|| SPINNER[self.spin_i])
    }
}

// Database location (XDG Data spec): $XDG_DATA_HOME/mypi/sessions.db,
// i.e. ~/.local/share/mypi/sessions.db by default. Legacy layout support:
// `~/.local/share/mypi` used to be the SQLite file itself — renamed to
// sessions.db on first open of the new layout.
fn db_path() -> std::path::PathBuf {
    crate::xdg::data_dir().join("sessions.db")
}

/// Migrate the legacy database file (a bare `mypi` file under
/// ~/.local/share) to the canonical `mypi/sessions.db` layout. No-op when
/// the legacy file is absent or already migrated.
fn migrate_legacy_db(base: &std::path::Path) {
    let legacy = base.join("mypi");
    if !legacy.is_file() {
        return;
    }
    // The legacy file **occupies the directory's name**, so: move the db
    // and its WAL/SHM companions out of the way, create the real
    // directory, then move them in as `sessions.db*`.
    let stash = base.join(".mypi-legacy-migrate");
    let _ = std::fs::remove_dir_all(&stash);
    std::fs::create_dir_all(&stash).ok();
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::rename(base.join(format!("mypi{suffix}")), stash.join(format!("db{suffix}")));
    }
    if let Err(e) = std::fs::create_dir_all(base.join("mypi")) {
        eprintln!("mypi: cannot create data dir: {e}");
        return;
    }
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::rename(stash.join(format!("db{suffix}")), base.join(format!("mypi/sessions.db{suffix}")));
    }
    let _ = std::fs::remove_dir_all(&stash);
}

// Session name for the statusline: an explicit /name wins; otherwise
// one is synthesized — first 7 chars of the first user message within the session.


fn display_name(app: &App) -> String {
    match app.session.session_name() {
        Some(n) => n.to_string(),
        None => app
            .session
            .transcript()
            .iter()
            .find_map(|e| match e {
                entry::Entry::User { content } => Some(content.chars().take(7).collect::<String>()),
                _ => None,
            })
            .unwrap_or_else(|| "新会话".into()),
    }
}


// Render one frame: sizes, scroll correction, widget drawing, hardware
// cursor placement. Shared by the pre-loop first paint and the signal
// pump — a stale first frame (or none at all) is how "blank until a
// keystroke" bugs happen.
fn draw_frame(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
    palette: &Palette,
    ctx_limit: u64,
    currency_symbol: &'static str,
    show_cost: bool,
) -> Result<crate::tui::layout::Layout> {
    let size = terminal.size()?;
    let wrapped = app.wrapped(size.width);
    let cwd_str = app.session.cwd().display().to_string();
    let cursor_char = app.editor.cursor();
    let spinner = app.spinner();

    // Viewport scrolling: minimal-displacement correction based on the
    // previous frame's viewport start. The height must be the one
    // **after subtracting the reserved area**, matching view's layout,
    // or the two sides disagree and the scroll window misaligns.
    let ch = tlayout::container_height(
        size.height.saturating_sub(app.reserved_height(size.height)),
        wrapped.len(),
    );
    let cursor_row = wrapped.locate(cursor_char).0;
    app.scroll = tlayout::adjust_scroll(app.scroll, wrapped.len(), ch, cursor_row);

    // Layout is computed once: `app` and `view` share the same sizes.
    let l = tlayout::compute(
        size.height,
        &wrapped,
        app.scroll,
        app.reserved_height(size.height),
        crate::tui::components::reserved::DEFAULT_MAX as u16,
    );
    let mut cursor_pos = (0u16, 0u16);
    let model_name = Config::display_name(&app.current_model.borrow()).to_string();
    let session_name = display_name(app);
    terminal.draw(|f| {
        // Modal takeover: the tree navigator draws over the whole
        // screen; base zones and the hardware cursor are skipped.
        if let Some(tp) = app.tree_pick.as_ref() {
            let lines = crate::tui::components::tree_picker::render(tp, size.width, size.height, palette);
            f.render_widget(ratatui::widgets::Paragraph::new(lines), f.area());
            return;
        }
        let mut vs = ViewState {
            history: app.session.transcript(),
            block_cache: &mut app.block_cache,
            chat_scroll: app.history.chat_scroll,
            scroll_pinned: app.history.scroll_pinned,
            show_reasoning: !app.history.reasoning_folded,
            tools_expanded: app.history.tools_expanded,
            // The live row is driven by the session, the only party that knows
            // whether the server is thinking or a tool is running.
            live: &app.session.stream_view().live,
            streaming: {
                let sv = app.session.stream_view();
                if sv.text.is_empty() { None } else { Some(sv.text.as_str()) }
            },
            wrapped: &wrapped,
            cursor_char,
            spinner,
            model_name: &model_name,
            session_name: &session_name,
            cwd: &cwd_str,
            git: app.git.as_ref(),
            ctx_tokens: app.tracker.last_prompt_tokens,
            ctx_limit,
            cost: app.tracker.total,
            currency_symbol,
            show_cost,
            palette: *palette,
            popup: app.completion.popup(),
            resume_pick: app.resume_pick.as_ref().map(|(v, i)| (&v[..], *i)),
        };
        cursor_pos = view::draw(f, &mut vs, &l);
    })?;

    // ---- hardware cursor ----
    // `Layout::cursor_y` guarantees the cursor stays inside the input
    // container and never tramples the reserved area; ratatui/crossterm
    // do no boundary checks, so this is the only gate.
    terminal.set_cursor_position(ratatui::layout::Position {
        x: cursor_pos.1.min(size.width.saturating_sub(1)),
        y: l.cursor_y(size.height, cursor_pos.0),
    })?;
    terminal.show_cursor()?;
    Ok(l)
}

// Run the TUI (blocking; Esc / Ctrl+C exits).
pub fn run_tui(cfg: Config, cli: crate::cli::Cli) -> Result<()> {
    // Session-scoped mutable config: /model and /switch both change the
    // current model, hence RefCell. Main thread only (Rc is not Send);
    // the background turn thread gets its own cloned Client.
    let cfg = std::rc::Rc::new(std::cell::RefCell::new(cfg));
    let rm = cfg.borrow().default_model()?;
    let provider = cfg
        .borrow()
        .models
        .providers
        .get(&rm.provider_name)
        .ok_or_else(|| anyhow::anyhow!("provider {} 未定义", rm.provider_name))?
        .clone();
    let model = rm.entry.clone();
    let api_key = cfg.borrow().resolve_key(&provider);
    let client = Client::new(&provider.base_url, &api_key, &model.id);
    let cost_cfg = model.cost;
    let currency_symbol = model.currency.symbol();
    let palette = Palette::from_config(&cfg.borrow());
    // Cost is computed **locally**: the gateway only reports token counts
    // (prompt/completion/cached), the program multiplies by the unit
    // prices the user wrote in models.yml. All-zero prices = "no price
    // sheet for this model" -> hide the spend column (there is nothing
    // meaningful to compute, not "the gateway didn't quote").
    let show_cost =
        (cost_cfg.input + cost_cfg.output + cost_cfg.cache_read + cost_cfg.cache_write) > 0.0;
    let current_model = std::rc::Rc::new(std::cell::RefCell::new(model.clone()));

    let chat = ChatContext::new().push(Message::System {
        content: "你是一个简洁的编程助手。用中文回答。".into(),
    });
    // Workspace license: launching from $HOME would license the whole
    // home directory for rm/mv — exactly what the guard exists to
    // prevent. Degrade to /tmp instead; the user can /cd out of it.
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let cwd = match std::env::current_dir() {
        Ok(d) if d == home => std::env::temp_dir(),
        Ok(d) => d,
        Err(_) => std::env::temp_dir(),
    };
    // Session service facade: owns turn resources + SessionState + the
    // event channel. The TUI holds it and the rx.
    let max_tokens = model.max_output_tokens.unwrap_or(4096) as u32;
    let (session, rx) = crate::server::Session::new(
        {
            migrate_legacy_db(&crate::xdg::data_dir());
            crate::server::SessionState::new(crate::store::Store::open(&db_path()).ok())
        },
        client,
        chat,
        max_tokens,
        cost_cfg,
        cwd.clone(),
    );
    let mut app = App::new(session, current_model);
    app.cfg = Some(cfg.clone());
    // `--resume`: open the session picker before the first frame (the
    // same surface /resume shows; Esc here simply starts a fresh session).
    if cli.resume
        && let Some(st) = app.session.store()
        && let Ok(items) = App::build_resume_items(st, &app.session.cwd())
        && !items.is_empty()
    {
        // No store / no sessions: fall through to a fresh session.
        app.resume_pick = Some((items, 0));
    }
    let ctx_limit = model.context_window;

    let mut terminal = ratatui::init();
    // Kitty keyboard protocol: ask the terminal to disambiguate escape
    // sequences so **Shift+Enter** arrives as Enter+SHIFT (newline) and
    // bare Enter as plain Enter (submit). Terminals without the
    // protocol silently ignore the escape codes — the pre-existing
    // fallbacks (Alt+Enter / Ctrl+J) keep working there.
    let _ = execute!(
        std::io::stdout(),
        ratatui::crossterm::event::PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES,
        )
    );
    // Enable bracketed paste: the terminal wraps pasted content in
    // \x1b[200~ ... \x1b[201~, so we get Event::Paste instead of every
    // line arriving as keystrokes.
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);
    // Enable mouse capture: wheel events scroll the history area. Side
    // effect: native text selection usually needs Shift+drag.
    let _ = execute!(std::io::stdout(), EnableMouseCapture);

    let result = (|| -> Result<()> {
        // Last frame's layout, for mouse zone hit-testing.
        let mut last_layout: Option<crate::tui::layout::Layout> = None;

        // Paint the first frame **before** blocking: the loop below is
        // signal-driven, and without an initial draw the user stares at
        // a blank screen until the first keystroke arrives.
        draw_frame(&mut terminal, &mut app, &palette, ctx_limit, currency_symbol, show_cost)?;

        // ---- signal pump: input thread + session events ----
        //
        // The loop is **signal-driven, not polled**: it blocks on
        // `sig_rx.recv()` and only wakes when something actually
        // happened. Two producers feed the loop:
        //   input thread  — forwards raw crossterm events as `Signal`s
        //   session drain — SessionEvents drained below, between signals
        // A burst of deltas marks the dirty bit repeatedly but repaints
        // once — coalescing is free because dirty is idempotent.
        let (sig_tx, sig_rx) = mpsc::channel::<crate::tui::signal::Signal>();
        // A second Sender handle for the session-event forwarder below.
        let sig_tx2 = sig_tx.clone();

        // Producer 1: raw terminal input, forwarded as signals.
        std::thread::spawn(move || {
            while let Ok(ev) = event::read() {
                let fwd = match ev {
                    Event::Key(k) if k.kind == KeyEventKind::Press => {
                        Some(crate::tui::signal::Signal::Key(k))
                    }
                    Event::Paste(s) => Some(crate::tui::signal::Signal::Paste(s)),
                    Event::Mouse(m) => Some(crate::tui::signal::Signal::Mouse(m)),
                    Event::Resize(_, _) => Some(crate::tui::signal::Signal::Resized),
                    _ => None,
                };
                if let Some(s) = fwd
                    && sig_tx.send(s).is_err()
                {
                    break; // main loop gone
                }
            }
        });

        // Producer 2: session events (deltas, tool calls, commits).
        // Forwarding them through the SAME channel is what makes the
        // loop truly signal-driven: a delta arriving wakes the loop and
        // repaints the live slot. (An earlier draft kept a separate
        // `rx` and drained it only after *keyboard* signals — the
        // stream never reached the screen unless the user typed.)
        std::thread::spawn(move || {
            for ev in rx {
                if sig_tx2.send(crate::tui::signal::Signal::Session(ev)).is_err() {
                    break; // main loop gone
                }
            }
        });

        let mut quit = false;
        while !quit {
            // Block until *something* happens. During streaming the
            // deltas themselves are the wake-ups; no POLL interval, no
            // idle wake-ups. Everything already queued behind the first
            // signal is folded into the same batch: N queued deltas =
            // one repaint, not N (coalescing for free).
            // The wait doubles as the spinner's heartbeat: while a turn
            // is streaming (including silent thinking phases, which
            // produce no deltas) wake at SPIN_INTERVAL to keep it
            // spinning; when idle block forever — no timers, no CPU.
            let first = if app.session.busy() {
                match sig_rx.recv_timeout(SPIN_INTERVAL) {
                    Ok(s) => Some(s),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
                }
            } else {
                Some(sig_rx.recv()?)
            };
            let mut batch = Vec::new();
            if let Some(s) = first {
                batch.push(s);
                // Fold everything already queued behind the first signal:
                // N queued deltas = one repaint, not N (coalescing free).
                while let Ok(more) = sig_rx.try_recv() {
                    batch.push(more);
                }
            }

            // ---- route the batch ----
            let mut batch_tools_ran = false;
            for sig in batch {
                match sig {
                crate::tui::signal::Signal::Key(k) => {
                    let term_w = terminal.size()?.width;
                    let action = translate_with(k, app.key_context(term_w));
                    quit = !app.apply(action, term_w);
                }
                crate::tui::signal::Signal::Paste(s) => {
                    // Paste also goes through `apply`.
                    //
                    // This used to be a **second** path: a direct
                    // `app.editor.insert_paste()` plus two lines of
                    // hand-copied epilogue, bypassing `apply`. It
                    // missed `input_history.on_edit()` — pasting
                    // while browsing history stuck in browse mode,
                    // and the next ↑ jumped to the entry before last
                    // instead of saving a draft. The two paths
                    // agreeing was pure luck; unified now, no drift
                    // possible.
                    let term_w = terminal.size()?.width;
                    quit = !app.apply(Action::Paste(s), term_w);
                }
                crate::tui::signal::Signal::Mouse(m) => {
                    // Hit-test the pointer row against the last frame's
                    // layout: only the history area scrolls (input and
                    // reserved ignore the wheel). Up unpin; back at 0
                    // re-pins. A modal turns the wheel into list scroll.
                    let chat_h = last_layout
                        .as_ref()
                        .map(|l: &crate::tui::layout::Layout| l.chat_height)
                        .unwrap_or(0);
                    if crate::tui::zones_impl::wheel_zone(m.row, chat_h, app.resume_pick.is_some())
                        == Some(crate::tui::zones::ZoneId::History)
                    {
                        match m.kind {
                            MouseEventKind::ScrollUp => crate::tui::zones_impl::wheel_step(&mut app.history, true, 3),
                            MouseEventKind::ScrollDown => crate::tui::zones_impl::wheel_step(&mut app.history, false, 3),
                            _ => {}
                        }
                    }
                }
                crate::tui::signal::Signal::Resized => {
                    // Widths changed: every cached wrap is invalid. The
                    // recompute below always reads terminal.size(), so
                    // nothing else to do — the repaint below is
                    // unconditional after any signal.
                }
                crate::tui::signal::Signal::Session(ev) => {
                    if app.session.ingest(ev) == crate::server::events::Change::ToolActivity {
                        batch_tools_ran = true;
                    }
                }
                }
            }

            // Environment checkpoint (event-driven): a tool result just
            // landed (tools are the only things that can move the working
            // tree — plain speech never triggers a refresh) or the cwd
            // moved. The 2s rate limit inside `refresh_env` absorbs
            // multi-tool bursts in a single round.
            if batch_tools_ran || app.env_cwd != app.session.cwd() {
                app.refresh_env();
            }

            // One spinner frame per loop pass: delta batches repaint
            // anyway, and SPIN_INTERVAL timeouts keep it alive through
            // silent thinking phases.
            app.advance_spinner();

            // ---- render one frame (shared with the pre-loop first draw) ----
            let l = draw_frame(&mut terminal, &mut app, &palette, ctx_limit, currency_symbol, show_cost)?;
            last_layout = Some(l);
        }
        Ok(())
    })();

    let _ = execute!(
        std::io::stdout(),
        ratatui::crossterm::event::PopKeyboardEnhancementFlags
    );
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    ratatui::restore();
    result
}

// Turn machinery (spawn runner, collect_turn, entries_to_context) lives
// in `crate::server::turn` now — the TUI is only a subscriber.
