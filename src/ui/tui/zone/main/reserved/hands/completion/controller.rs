//! The completion surface: the popup, and the memo of the word it was computed for.

use std::path::PathBuf;

use super::engine::{self, Completion, CompletionPopup, Look, Scan};
use crate::ui::lang::Lang;
use crate::ui::tui::action::{Key, KeyCode, Mods};
use crate::ui::tui::theme::Accent;
use crate::ui::tui::zone::Claim;
use crate::ui::tui::zone::main::desk::Reply;
use crate::ui::tui::zone::main::input::command;
use crate::ui::tui::zone::main::reserved::hand::{Hand, Report};
use crate::ui::tui::zone::main::reserved::pools::Pools;

/// What the controller reads to compute candidates.
pub struct InputCtx<'a> {
    pub text: &'a str,
    pub cursor: usize,
    pub cwd: &'a std::path::Path,
    pub home: &'a std::path::Path,
    pub pools: Option<&'a Pools>,
    pub lang: Lang,
}

pub struct CompletionController {
    popup: CompletionPopup,
    /// The word the current candidates were computed for.
    last_word: Option<(usize, String)>,
    /// The spot the user dismissed (Esc): the same word stays closed.
    dismissed: Option<(usize, String)>,
    /// The current word's candidates come from the name pools.
    names: bool,
}

impl CompletionController {
    pub fn new() -> Self {
        Self {
            popup: CompletionPopup::default(),
            last_word: None,
            dismissed: None,
            names: false,
        }
    }

    pub fn popup(&self) -> &CompletionPopup {
        &self.popup
    }

    pub fn is_open(&self) -> bool {
        self.popup.is_open()
    }

    /// The current word's candidates are names: the pools must land before a height is frozen.
    pub fn wants_names(&self) -> bool {
        self.names
    }

    /// The height this completion locked, from its first level's whole list.
    pub fn reserved_height(&self) -> Option<usize> {
        self.popup.locked_height()
    }

    /// Recompute candidates for the word the cursor is in.
    pub fn refresh(&mut self, cx: &InputCtx, scan: &mut Scan) {
        let look = Look {
            cwd: cx.cwd,
            home: cx.home,
            pools: cx.pools,
            lang: cx.lang,
        };
        let found = engine::suggest(cx.text, cx.cursor, &look, scan);
        let spot = (found.from, found.word.clone());
        self.names = found.names;
        // A dismissal covers one word only: the word moved, so it is spent.
        if self.dismissed.as_ref().is_some_and(|d| *d != spot) {
            self.dismissed = None;
        }
        // The same word (a cursor move) leaves the popup as it stands.
        if self.last_word.as_ref() == Some(&spot) && self.popup.is_open() {
            return;
        }
        // The user dismissed this spot: candidates do not reopen it.
        if self.dismissed.as_ref() == Some(&spot) {
            return;
        }
        self.last_word = Some(spot);
        if found.items.is_empty() || leaf(&found.items, &found.word, found.from == 0) {
            self.close();
        } else {
            self.popup
                .open(found.items, found.from, cx.cursor, found.total);
        }
    }

    /// Tab: open, extend to the candidates' common prefix, or confirm.
    pub fn on_tab(&mut self, cx: &InputCtx, scan: &mut Scan) -> Option<engine::CompletionAction> {
        if !self.popup.is_open() {
            self.refresh(cx, scan);
            if !self.popup.is_open() {
                return None;
            }
            if self.popup.items().len() == 1 {
                return Some(self.popup.accept());
            }
            return None;
        }
        let word = self
            .last_word
            .as_ref()
            .map(|(_, w)| w.clone())
            .unwrap_or_default();
        if let Some(action) = self.popup.accept_common_prefix(&word) {
            return Some(action);
        }
        Some(self.popup.accept())
    }

    pub fn move_selection(&mut self, delta: isize) {
        self.popup.move_selection(delta);
    }

    /// Esc: close, and remember this word so it does not reopen while the word stands.
    pub fn dismiss(&mut self) {
        if self.popup.is_open() {
            self.dismissed = self.last_word.clone();
        }
        self.close();
    }

    /// Close and drop the memo.
    pub fn close(&mut self) {
        self.popup.close();
        self.last_word = None;
    }
}

impl Default for CompletionController {
    fn default() -> Self {
        Self::new()
    }
}

/// A completion that is done: one candidate already spells the word, or the word names a
/// command that takes no argument. A directory's `/` is decoration: losing it ends the word.
fn leaf(items: &[Completion], word: &str, at_line_start: bool) -> bool {
    if items.iter().any(|c| c.insert.trim_end_matches('/') == word) {
        return true;
    }
    at_line_start && command::lookup(word).is_some_and(|meta| meta.args.is_empty())
}

/// The completion hand: the reserved zone hires it, it borrows the popup's keys.
pub struct CompletionService {
    controller: CompletionController,
    /// The workspace the paths complete against, taken from the view.
    cwd: PathBuf,
    /// `~` resolves to it, for lookups only.
    home: PathBuf,
    /// The model and profile names; asked again on every arrival, kept for the popup.
    pools: Option<Pools>,
    /// This arrival has no pools yet: the desk must not freeze a height before they land.
    awaiting: bool,
    /// The cursor's column, pushed by the input zone each frame.
    anchor_col: usize,
    /// The editor's text and cursor, as last fed.
    last: (String, usize),
    scan: Scan,
    /// The language the command details read in.
    lang: Lang,
}

impl CompletionService {
    /// The hand builds itself: the workspace and the name pools come from the view,
    /// `~` resolves against the environment it runs in.
    pub fn new() -> Self {
        Self {
            controller: CompletionController::new(),
            cwd: PathBuf::new(),
            home: std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default(),
            pools: None,
            awaiting: false,
            anchor_col: 0,
            last: (String::new(), 0),
            scan: Scan::default(),
            lang: Lang::default(),
        }
    }

    /// The language the command details read in; the next edit reads it.
    pub fn set_lang(&mut self, lang: Lang) {
        self.lang = lang;
    }

    /// Recompute candidates after an input change.
    fn refresh(&mut self, text: &str, cursor: usize) {
        let cx = InputCtx {
            text,
            cursor,
            cwd: &self.cwd,
            home: &self.home,
            pools: self.pools.as_ref(),
            lang: self.lang,
        };
        self.controller.refresh(&cx, &mut self.scan);
        self.awaiting = self.pools.is_none() && self.controller.wants_names();
    }

    /// Tab semantics for the editor's completion key.
    fn on_tab(&mut self) -> engine::CompletionAction {
        let cx = InputCtx {
            text: &self.last.0,
            cursor: self.last.1,
            cwd: &self.cwd,
            home: &self.home,
            pools: self.pools.as_ref(),
            lang: self.lang,
        };
        self.controller
            .on_tab(&cx, &mut self.scan)
            .unwrap_or(engine::CompletionAction::None)
    }
}

impl Default for CompletionService {
    fn default() -> Self {
        Self::new()
    }
}

impl Hand for CompletionService {
    fn arrived(&mut self) {
        self.awaiting = false;
    }

    fn place(&mut self, cwd: &str) {
        if self.cwd.as_path() != std::path::Path::new(cwd) {
            self.cwd = PathBuf::from(cwd);
        }
    }

    fn pools(&mut self, pools: &Pools) {
        self.pools = Some(pools.clone());
        self.awaiting = false;
        let (text, cursor) = self.last.clone();
        self.refresh(&text, cursor);
    }

    fn ready(&self) -> bool {
        !self.awaiting
    }

    fn report(&self) -> Report {
        Report {
            rows: self.controller.reserved_height().unwrap_or(1) as u16,
            keys: COMPLETION_KEYS,
        }
    }

    fn stays(&self) -> bool {
        // A hand still waiting for the names holds the seat: dropping it would drop the popup.
        self.awaiting || self.controller.is_open()
    }

    fn text(&mut self, text: &str, cursor: usize) {
        self.last = (text.to_string(), cursor);
        self.refresh(text, cursor);
    }

    fn col(&mut self, col: u16) {
        self.anchor_col = engine::anchor_column(&self.last.0, self.last.1, col as usize);
    }

    fn on_key(&mut self, key: &Key) -> Option<Reply> {
        match key.code {
            KeyCode::Esc => {
                self.controller.dismiss();
                None
            }
            KeyCode::Up => {
                self.controller.move_selection(-1);
                None
            }
            KeyCode::Down => {
                self.controller.move_selection(1);
                None
            }
            KeyCode::Tab | KeyCode::Enter => match self.on_tab() {
                engine::CompletionAction::Replace { from, to, text } => {
                    Some(Reply::Replace { from, to, text })
                }
                engine::CompletionAction::None => None,
            },
            _ => None,
        }
    }

    fn draw(&self, width: u16, rows: u16, accent: &Accent) -> Vec<ratatui::text::Line<'static>> {
        super::popup::render(
            self.controller.popup(),
            width,
            usize::from(rows),
            self.anchor_col,
            accent,
        )
        .lines
    }

    fn left(&mut self) {
        self.controller.close();
        self.awaiting = false;
    }
}

/// Keys the popup borrows: pick a row, confirm, dismiss.
const COMPLETION_KEYS: &[Claim] = &[
    Claim::Key(Key {
        code: KeyCode::Esc,
        mods: Mods {
            ctrl: false,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::Up,
        mods: Mods {
            ctrl: false,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::Down,
        mods: Mods {
            ctrl: false,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::Tab,
        mods: Mods {
            ctrl: false,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::Enter,
        mods: Mods {
            ctrl: false,
            alt: false,
            shift: false,
        },
    }),
];
