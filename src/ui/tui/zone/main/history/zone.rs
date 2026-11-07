//! The history zone: what it claims, what it holds, and the contract it answers.

use std::collections::HashSet;

use serde_json::{Value, json};

use super::local::section::{Idx, Local};
use super::local::slot::{Level, Life, SlotKind};
use super::prefetch::{Run, walk};
use super::product::{Opts, Products, Task, TodoPanel};
use super::view::{Dir, Scene, View};
use super::{view, wire};

use super::super::pane::Pane;
use crate::ui::tui::TodoFold;
use crate::ui::tui::action::{Action, Key, KeyCode, Mods};
use crate::ui::tui::theme::Accent;
use crate::ui::tui::zone::{Claim, Frame, Kind, Request, Shout, SubZone, TermSize, Turns, name};

/// The keys history consumes. The input zone yields Ctrl+arrows to it.
pub const CLAIMS: &[Claim] = &[
    Claim::Key(Key {
        code: KeyCode::Char('t'),
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::Char('o'),
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::Up,
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::Down,
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::PageUp,
        mods: Mods {
            ctrl: false,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::PageDown,
        mods: Mods {
            ctrl: false,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::Home,
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: true,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::End,
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: true,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::PageUp,
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::PageDown,
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: false,
        },
    }),
    Claim::Wheel,
    Claim::WheelShift,
];

/// The classes the zone reads.
const KINDS: &[Kind] = &[Kind::Call, Kind::Stream, Kind::Notice];

/// The sight line until the config answers.
const DEFAULT_FOCUS_LINE: u16 = 3;

/// Blocks kept in the product layer until the config answers.
const DEFAULT_MARGIN: usize = 64;

/// Rows a card head or tail may take until the config answers.
const DEFAULT_FOLD: usize = 10;

/// Rows one wheel notch walks until the config answers.
const DEFAULT_SCROLL: usize = 10;

/// The config keys this zone asks for once per attachment.
pub(super) const CONFIG_KEYS: &[&str] = &[
    "config.ui.tui.zone.main.history.focusLine",
    "config.ui.tui.zone.main.history.cardFoldBudget",
    "config.ui.tui.zone.main.history.cacheMargin",
    "config.ui.tui.zone.main.history.scrollHeight",
    "config.ui.tui.zone.main.history.todoFold",
];

/// The chat history: what the reader sees of the conversation.
pub struct HistoryZone {
    /// The local section: render order and the content behind it.
    pub(super) local: Local,
    /// The block cache.
    pub(super) products: Products,
    /// The laid segments around the sight line.
    pub(super) run: Run,
    /// The window.
    pub(super) view: View,
    /// The checklist's own product.
    pub(super) todo_panel: TodoPanel,
    pub(super) todo: Vec<Task>,
    pub(super) todo_fold: TodoFold,
    /// The last width the layout was built at.
    pub(super) width: usize,
    pub(super) thinking: bool,
    pub(super) tool: bool,
    pub(super) fold: usize,
    pub(super) scroll_height: usize,
    pub(super) placeholder: String,
    pub(super) cwd: String,
    /// The structural window as it last stood.
    pub(super) structure: Turns,
    /// The id ranges asked for and not yet answered.
    pub(super) asking: HashSet<[i64; 2]>,
    /// The newest block id this zone has numbered.
    pub(super) landed: i64,
    /// A fresh attach lays the structure out again.
    pub(super) needs_layout: bool,
    /// The block the sight line is waiting for, once it and its content are here.
    pub(super) pending_focus: Option<i64>,
    /// What this frame owes the session.
    pub(super) outbox: Vec<Request>,
    /// What this frame owes the other zones.
    pub(super) shouts: Vec<Shout>,
}

impl Default for HistoryZone {
    fn default() -> Self {
        let mut zone = HistoryZone {
            local: Local::default(),
            products: Products::default(),
            run: Run::default(),
            view: View::default(),
            todo_panel: TodoPanel::default(),
            todo: Vec::new(),
            todo_fold: TodoFold::default(),
            width: 1,
            thinking: false,
            tool: false,
            fold: DEFAULT_FOLD,
            scroll_height: DEFAULT_SCROLL,
            placeholder: "thinking…".to_string(),
            cwd: String::new(),
            structure: Turns::default(),
            asking: HashSet::new(),
            landed: 0,
            needs_layout: false,
            pending_focus: None,
            outbox: Vec::new(),
            shouts: Vec::new(),
        };
        zone.view.set_focus_line(DEFAULT_FOCUS_LINE);
        zone.products.set_margin(DEFAULT_MARGIN);
        zone
    }
}

impl HistoryZone {
    /// Run one operation that needs the window and the run at once.
    fn with_scene<R>(&mut self, f: impl FnOnce(&mut View, &mut Scene) -> R) -> R {
        let HistoryZone {
            local,
            products,
            run,
            view,
            width,
            thinking,
            tool,
            placeholder,
            cwd,
            fold,
            ..
        } = self;
        let opts = Opts {
            thinking: *thinking,
            tool: *tool,
            placeholder: placeholder.as_str(),
            cwd: cwd.as_str(),
            fold: *fold,
        };
        let mut scene = Scene {
            local,
            products,
            run,
            opts: &opts,
            width: *width,
        };
        f(view, &mut scene)
    }

    /// One frame: the window, then the checklist pinned under it.
    pub fn render_rows(
        &mut self,
        term_w: u16,
        accent: Accent,
    ) -> Vec<ratatui::text::Line<'static>> {
        let width = usize::from(term_w.max(1));
        let budget = usize::from(self.view.rows()) / 2;
        let pinned = self
            .todo_panel
            .rows(&self.todo, self.todo_fold, width, budget)
            .to_vec();
        self.with_scene(|view, scene| view::window::rows(view, scene, &pinned, accent))
    }

    /// A line this front end has to say, on its own slot.
    pub fn notice(&mut self, text: String, level: Level, life: Life) {
        self.local.notice(text, level, life);
    }

    /// Put a block's head one row below the sight line, asking for its round when needed.
    pub fn focus(&mut self, block: i64) {
        self.pending_focus = Some(block);
        self.settle_focus();
    }

    /// Land the pending focus once the block and its content are in hand.
    pub(super) fn settle_focus(&mut self) {
        let Some(block) = self.pending_focus else {
            return;
        };
        if !self.local.has(block) {
            if let Some(turn) = self.turn_of(block) {
                self.ask_turn(turn);
            }
            return;
        }
        let Some(idx) = self.local.idx_of(block) else {
            return;
        };
        self.pending_focus = None;
        self.with_scene(|view, scene| view::focus::focus(view, scene, idx));
    }

    /// `/new`: drop this conversation's section; the next attach asks for the new one.
    pub fn reset(&mut self) {
        self.destroy();
    }

    /// The local section goes, and the run, the checklist and the window with it.
    pub(super) fn destroy(&mut self) {
        self.local.clear();
        self.products.clear();
        self.run.clear();
        self.structure.clear();
        self.asking.clear();
        self.todo.clear();
        self.todo_panel.changed();
        self.view.follow();
        self.pending_focus = None;
        self.landed = 0;
        self.needs_layout = false;
    }

    /// Frames this zone asked for, taken by the host.
    pub(in crate::ui::tui::zone::main) fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.outbox)
    }

    /// The block under the sight line, or the newest block when following.
    pub(super) fn anchor_block(&self) -> Option<i64> {
        match self.view.at() {
            Some(at) => match self.local.slot(at.idx)?.kind {
                SlotKind::Block(id) => Some(id),
                _ => None,
            },
            None => {
                let num = self.local.newest()?;
                match self.local.at(num)?.kind {
                    SlotKind::Block(id) => Some(id),
                    _ => None,
                }
            }
        }
    }

    /// The slot under the sight line, or the newest when following.
    fn anchor_slot(&self) -> Option<Idx> {
        match self.view.at() {
            Some(at) => Some(at.idx),
            None => self.local.newest().map(Idx::Fixed),
        }
    }

    /// One whole slot older or newer: its head under the sight line.
    pub(super) fn step_slot(&mut self, dir: Dir) {
        let Some(from) = self.anchor_slot() else {
            return;
        };
        let mut cursor = match dir {
            Dir::Older => walk::older(&self.local, from),
            Dir::Newer => walk::newer(&self.local, from),
        };
        while let Some(idx) = cursor {
            if self.run.head(idx).is_some() || self.local.block(idx).is_some() {
                self.jump(idx);
                return;
            }
            cursor = match dir {
                Dir::Older => walk::older(&self.local, idx),
                Dir::Newer => walk::newer(&self.local, idx),
            };
        }
        if dir == Dir::Newer {
            self.view.follow();
        }
    }

    /// Put one slot's head under the sight line.
    pub(super) fn jump(&mut self, idx: Idx) {
        self.with_scene(|view, scene| view::focus::focus(view, scene, idx));
    }

    /// One whole round older or newer: its first block under the sight line.
    pub(super) fn turn_step(&mut self, dir: Dir) {
        let Some(block) = self.anchor_block() else {
            return;
        };
        let Some(turn) = self.turn_of(block) else {
            return;
        };
        let rounds = self.structure.rounds();
        let Some(at) = rounds.iter().position(|round| round.id == turn) else {
            return;
        };
        let next = match dir {
            Dir::Older => at + 1,
            Dir::Newer => match at.checked_sub(1) {
                Some(index) => index,
                None => return,
            },
        };
        let Some([lo, _]) = rounds.get(next).and_then(|round| round.ranges.first()) else {
            return;
        };
        self.focus(*lo);
    }

    /// Put the sight line on the first or last block of the anchor's round.
    pub(super) fn align(&mut self, last: bool) {
        let Some(block) = self.anchor_block() else {
            return;
        };
        let Some(turn) = self.turn_of(block) else {
            return;
        };
        let Some(id) = self
            .structure
            .rounds()
            .iter()
            .find(|round| round.id == turn)
            .and_then(|round| {
                let ids = round.ranges.iter().flat_map(|[lo, hi]| *lo..=*hi);
                if last { ids.max() } else { ids.min() }
            })
        else {
            return;
        };
        self.focus(id);
    }

    /// The oldest round's first block.
    pub(super) fn first(&mut self) {
        let id = self
            .structure
            .rounds()
            .last()
            .and_then(|round| round.ranges.first())
            .map(|[lo, _]| *lo);
        if let Some(id) = id {
            self.focus(id);
        }
    }
}

impl SubZone for HistoryZone {
    fn claims(&self) -> &'static [Claim] {
        CLAIMS
    }

    fn kinds(&self) -> &'static [Kind] {
        KINDS
    }

    fn deliver_event(&mut self, frame: &Frame, _size: TermSize) -> Vec<Request> {
        wire::read_frame(self, frame);
        self.take_requests()
    }

    fn deliver_action(&mut self, action: &Action) -> bool {
        let handled = match action {
            Action::ScrollUp { shift: false } => {
                self.scroll(Dir::Older, self.scroll_height.max(1));
                true
            }
            Action::ScrollDown { shift: false } => {
                self.scroll(Dir::Newer, self.scroll_height.max(1));
                true
            }
            Action::ScrollUp { shift: true } => {
                self.turn_step(Dir::Older);
                true
            }
            Action::ScrollDown { shift: true } => {
                self.turn_step(Dir::Newer);
                true
            }
            Action::Paste(_) => false,
            Action::Key(key) => match key.code {
                KeyCode::Char('t') if key.mods.ctrl => {
                    self.thinking = !self.thinking;
                    self.products.clear();
                    self.run.clear();
                    true
                }
                KeyCode::Char('o') if key.mods.ctrl => {
                    self.tool = !self.tool;
                    self.products.drop_fixed();
                    self.run.clear();
                    true
                }
                KeyCode::Up if key.mods.ctrl => {
                    self.step_slot(Dir::Older);
                    true
                }
                KeyCode::Down if key.mods.ctrl => {
                    self.step_slot(Dir::Newer);
                    true
                }
                KeyCode::PageUp if key.mods.ctrl => {
                    self.first();
                    true
                }
                KeyCode::PageDown if key.mods.ctrl => {
                    self.view.follow();
                    true
                }
                KeyCode::PageUp => {
                    let rows = usize::from(self.view.rows());
                    self.scroll(Dir::Older, rows);
                    true
                }
                KeyCode::PageDown => {
                    let rows = usize::from(self.view.rows());
                    self.scroll(Dir::Newer, rows);
                    true
                }
                KeyCode::Home if key.mods.ctrl && key.mods.shift => {
                    self.align(false);
                    true
                }
                KeyCode::End if key.mods.ctrl && key.mods.shift => {
                    self.align(true);
                    true
                }
                _ => false,
            },
        };
        if handled {
            self.ensure_around();
        }
        handled
    }

    fn attach(&mut self, _session: i64, _size: TermSize) {
        self.destroy();
        self.needs_layout = true;
        self.view.follow();
        for key in CONFIG_KEYS {
            self.outbox.push(Request::Config {
                key: (*key).to_string(),
            });
        }
    }

    fn detach(&mut self) {
        self.destroy();
    }

    fn structure(&mut self, snapshot: &Turns) {
        wire::apply_structure(self, snapshot);
        self.shouts
            .push(Shout::new(name::BUSY, json!({"on": false})));
    }

    fn take_shouts(&mut self) -> Vec<Shout> {
        std::mem::take(&mut self.shouts)
    }

    fn hear(&mut self, event: &str, content: &Value) {
        match event {
            name::CWD => {
                if let Some(path) = content.get("path").and_then(Value::as_str)
                    && self.cwd != path
                {
                    self.cwd.clear();
                    self.cwd.push_str(path);
                }
            }
            name::ECHO => {
                if let Some(text) = content.get("text").and_then(Value::as_str) {
                    self.local.live_mut().echo(text);
                }
            }
            name::RETRACT => {
                self.local.live_mut().retract();
                let reason = content
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("the message never opened a round");
                self.notice(reason.to_string(), Level::Warn, Life::Short);
            }
            _ => {}
        }
    }
}

impl Pane for HistoryZone {
    fn assign(&mut self, term: TermSize, rows: u16) {
        self.view.assign(rows);
        let width = usize::from(term.cols.max(1));
        if self.width != width {
            self.width = width;
            self.run.set_width(width, &mut self.products);
        }
    }

    fn on_height_changed(&mut self, rows: u16) {
        self.view.assign(rows);
    }
}

impl HistoryZone {
    /// One scroll by rows.
    pub(super) fn scroll(&mut self, dir: Dir, rows: usize) {
        self.with_scene(|view, scene| view::scroll::step(view, scene, dir, rows));
    }
}
