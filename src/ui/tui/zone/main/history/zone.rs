//! The history zone: what it claims, what it holds, and the contract it answers.

use std::collections::HashSet;

use serde_json::Value;

use super::focus::{Focus, Target};
use super::local::section::{Idx, Local};
use super::local::slot::{Level, Life, SlotKind};
use super::prefetch::{Range, Run, Start, walk};
use super::product::{Opts, Products, Task, TodoPanel};
use super::{display, wire};

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
    /// The laid segments around the screen.
    pub(super) run: Run,
    /// Where the screen starts.
    pub(super) range: Range,
    /// The checklist's own product.
    pub(super) todo_panel: TodoPanel,
    pub(super) todo: Vec<Task>,
    pub(super) todo_fold: TodoFold,
    /// The last width the layout was built at.
    pub(super) width: usize,
    /// The rows the host granted.
    pub(super) height: u16,
    pub(super) focus_line: usize,
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
    /// The place the window is waiting to point at, once its content is here.
    pub(super) pending_focus: Option<Target>,
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
            range: Range::default(),
            todo_panel: TodoPanel::default(),
            todo: Vec::new(),
            todo_fold: TodoFold::default(),
            width: 1,
            height: 0,
            focus_line: usize::from(DEFAULT_FOCUS_LINE),
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
        zone.products.set_margin(DEFAULT_MARGIN);
        zone
    }
}

impl HistoryZone {
    /// Run one operation that needs the local section, the products, the run and the range at once.
    pub(super) fn with_draw<R>(
        &mut self,
        f: impl FnOnce(&Local, &mut Products, &mut Run, &mut Range, &Opts<'_>) -> R,
    ) -> R {
        let HistoryZone {
            local,
            products,
            run,
            range,
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
        f(local, products, run, range, &opts)
    }

    /// The screen: the transcript, then the checklist pinned under it.
    pub fn render_rows(
        &mut self,
        term_w: u16,
        accent: Accent,
    ) -> Vec<ratatui::text::Line<'static>> {
        let width = usize::from(term_w.max(1));
        let budget = usize::from(self.height) / 2;
        let pinned = {
            let HistoryZone {
                todo_panel,
                todo,
                todo_fold,
                ..
            } = self;
            todo_panel.rows(todo, *todo_fold, width, budget).to_vec()
        };
        let body = usize::from(self.height).saturating_sub(pinned.len()).max(1);
        let spend = self.with_draw(|local, products, run, range, opts| {
            range.rows(run, local, products, opts, body)
        });
        let top = spend.top;
        self.with_draw(|_, _, run, _, _| run.trim(top, top + body));
        for block in &spend.missing {
            if let Some(turn) = self.turn_of(*block) {
                self.ask_turn(turn);
            }
        }
        let mut out = display::lines(&spend.rows, accent);
        out.extend(pinned.iter().map(|row| row.paint(accent)));
        out
    }

    /// One position grew: its last segment is laid again, and its exchange's other half with it.
    pub(super) fn touch_turn(&mut self, at: usize) {
        let idx = Idx::Turn(at as u32);
        let other = self.local.sibling(idx);
        self.with_draw(|local, products, run, _, opts| {
            run.touch(local, products, opts, idx);
            if let Some(other) = other {
                run.touch(local, products, opts, other);
            }
        });
    }

    /// One fixed position's content changed: its rows go.
    pub(super) fn forget(&mut self, num: i64) {
        self.run.forget(Idx::Fixed(num));
    }

    /// A line this front end has to say, on its own slot.
    pub fn notice(&mut self, text: String, level: Level, life: Life) {
        self.local.notice(text, level, life);
    }

    /// `/new`: drop this conversation's section; the next attach asks for the new one.
    pub fn reset(&mut self) {
        self.destroy();
    }

    /// The local section goes, and the run, the checklist and the range with it.
    pub(super) fn destroy(&mut self) {
        self.local.clear();
        self.products.clear();
        self.run.clear();
        self.range.follow();
        self.structure.clear();
        self.asking.clear();
        self.todo.clear();
        self.todo_panel.changed();
        self.pending_focus = None;
        self.landed = 0;
        self.needs_layout = false;
    }

    /// Frames this zone asked for, taken by the host.
    pub(in crate::ui::tui::zone::main) fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.outbox)
    }

    /// One whole slot older or newer: its head under the sight line.
    pub(super) fn step_slot(&mut self, older: bool) {
        let Some(from) = self.anchor_slot() else {
            return;
        };
        let mut cursor = if older {
            walk::older(&self.local, from)
        } else {
            walk::newer(&self.local, from)
        };
        while let Some(idx) = cursor {
            if self.local.holds_block(idx) {
                self.focus_at(idx, older);
                return;
            }
            cursor = if older {
                walk::older(&self.local, idx)
            } else {
                walk::newer(&self.local, idx)
            };
        }
        if !older {
            self.range.follow();
        }
    }

    /// Point at the newest finished round's head.
    fn focus_newest_round(&mut self) {
        if let Some(turn) = self.structure.rounds().first().map(|round| round.id) {
            self.focus(Focus::Round(turn));
        }
    }

    /// One whole round older or newer: its head under the sight line.
    pub(super) fn turn_step(&mut self, older: bool) {
        if matches!(self.range.start(), Start::Latest) {
            if !older {
                self.range.follow();
                return;
            }
            if self.running_head().is_some() {
                self.focus(Focus::Running);
            } else {
                self.focus_newest_round();
            }
            return;
        }
        let Some(slot) = self.anchor_slot() else {
            return;
        };
        if let Idx::Turn(at) = slot {
            if !older {
                self.range.follow();
                return;
            }
            match self.running_head() {
                Some(head) if head != at => self.focus(Focus::Running),
                _ => self.focus_newest_round(),
            }
            return;
        }
        let Idx::Fixed(num) = slot else {
            return;
        };
        let direct = matches!(
            self.local.slot(slot).map(|slot| &slot.kind),
            Some(SlotKind::Block(_))
        );
        let Some(block) = self.block_near(num, older) else {
            return;
        };
        let Some(turn) = self.turn_of(block) else {
            return;
        };
        let next = {
            let rounds = self.structure.rounds();
            let Some(at) = rounds.iter().position(|round| round.id == turn) else {
                return;
            };
            if direct {
                if older {
                    rounds.get(at + 1).map(|round| round.id)
                } else {
                    at.checked_sub(1)
                        .and_then(|index| rounds.get(index))
                        .map(|round| round.id)
                }
            } else {
                Some(turn)
            }
        };
        match next {
            Some(turn) => self.focus(Focus::Round(turn)),
            None if !older => self.range.follow(),
            None => {}
        }
    }

    /// Put the sight line on the first or last block of the anchor's round.
    pub(super) fn align(&mut self, last: bool) {
        let Some(slot) = self.anchor_slot() else {
            return;
        };
        let Idx::Fixed(num) = slot else {
            return;
        };
        let Some(block) = self.block_near(num, true) else {
            return;
        };
        let Some(turn) = self.turn_of(block) else {
            return;
        };
        if !last {
            self.focus(Focus::Round(turn));
            return;
        }
        let id = {
            let rounds = self.structure.rounds();
            let Some(round) = rounds.iter().find(|round| round.id == turn) else {
                return;
            };
            round.ranges.iter().flat_map(|[lo, hi]| *lo..=*hi).max()
        };
        if let Some(id) = id {
            self.focus_block(id, false);
        }
    }

    /// The oldest round's head.
    pub(super) fn first(&mut self) {
        if let Some(turn) = self.structure.rounds().last().map(|round| round.id) {
            self.focus(Focus::Round(turn));
        }
    }

    /// One scroll by rows.
    pub(super) fn scroll(&mut self, older: bool, rows: usize) {
        let height = usize::from(self.height).max(1);
        let missing = self.with_draw(|local, products, run, range, opts| {
            range.scroll(run, local, products, opts, height, older, rows)
        });
        for block in missing {
            if let Some(turn) = self.turn_of(block) {
                self.ask_turn(turn);
            }
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
                self.scroll(true, self.scroll_height.max(1));
                true
            }
            Action::ScrollDown { shift: false } => {
                self.scroll(false, self.scroll_height.max(1));
                true
            }
            Action::ScrollUp { shift: true } => {
                self.turn_step(true);
                true
            }
            Action::ScrollDown { shift: true } => {
                self.turn_step(false);
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
                    self.step_slot(true);
                    true
                }
                KeyCode::Down if key.mods.ctrl => {
                    self.step_slot(false);
                    true
                }
                KeyCode::PageUp if key.mods.ctrl => {
                    self.first();
                    true
                }
                KeyCode::PageDown if key.mods.ctrl => {
                    self.range.follow();
                    true
                }
                KeyCode::PageUp => {
                    let rows = usize::from(self.height);
                    self.scroll(true, rows);
                    true
                }
                KeyCode::PageDown => {
                    let rows = usize::from(self.height);
                    self.scroll(false, rows);
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
        self.range.follow();
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
                    self.local.running_mut().echo(text);
                    self.shouts
                        .push(Shout::new(name::TICK, serde_json::json!({})));
                }
            }
            name::RETRACT => {
                self.local.running_mut().retract();
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
        self.height = rows;
        let width = usize::from(term.cols.max(1));
        if self.width != width {
            self.width = width;
            self.with_draw(|_, products, run, _, _| run.set_width(width, products));
        }
    }

    fn on_height_changed(&mut self, rows: u16) {
        self.height = rows;
    }
}
