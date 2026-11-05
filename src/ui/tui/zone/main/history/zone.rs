//! The history zone: what it claims, what it holds, and the contract it answers.

use std::collections::{BTreeMap, HashSet};

use serde_json::{Value, json};

use super::display::{Dir, Display};
use super::local::{Level, Life, Local, SlotKind};
use super::paint::todo::Task;
use super::prefetch::Prefetch;
use super::product::{Products, TodoPanel};
use super::wire::decode::Block;
use super::wire::live::Live;

use super::super::pane::Pane;
use crate::ui::tui::TodoFold;
use crate::ui::tui::action::{Action, Key, KeyCode, Mods};
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
const CONFIG_KEYS: &[&str] = &[
    "config.ui.tui.zone.main.history.focusLine",
    "config.ui.tui.zone.main.history.cardFoldBudget",
    "config.ui.tui.zone.main.history.cacheMargin",
    "config.ui.tui.zone.main.history.scrollHeight",
    "config.ui.tui.zone.main.history.todoFold",
];

/// The chat history: what the reader sees of the conversation.
pub struct HistoryZone {
    /// Every landed block this front end holds, by id.
    pub(super) blocks: BTreeMap<i64, Block>,
    /// Ranges already asked for, so one turn is not asked twice.
    pub(super) asking: HashSet<[i64; 2]>,
    /// The structural window, as last handed down.
    pub(super) structure: Turns,
    /// The latest block id that has taken a slot.
    pub(super) landed: i64,
    /// What the conversation is made of, in order.
    pub(super) local: Local,
    /// The tail still growing.
    pub(super) live: Live,
    /// The product layer: a block to pieces, kept by block.
    pub(super) products: Products,
    /// The prefetch layer: the laid rows around the sight line.
    pub(super) prep: Prefetch,
    /// The display layer: the window and the scroll.
    pub(super) disp: Display,
    /// The checklist's own product.
    pub(super) todo_panel: TodoPanel,
    /// The model's checklist, as the session replays it.
    pub(super) todo: Vec<Task>,
    pub(super) todo_fold: TodoFold,
    /// The thinking chain's own copy.
    pub(super) thinking: bool,
    /// Tool exchanges' own copy.
    pub(super) tool: bool,
    pub(super) placeholder: String,
    /// Rows a card head or a card tail may take before it folds.
    pub(super) fold: usize,
    /// Screen rows one wheel notch walks.
    pub(super) scroll_height: usize,
    /// Where the tools ran, for shortening paths.
    pub(super) cwd: String,
    /// A fresh attach lays the structure out again.
    pub(super) needs_layout: bool,
    /// A spot to put the sight line on, once its block has a slot.
    pub(super) pending_focus: Option<(i64, i64)>,
    /// What this frame owes the session.
    pub(super) outbox: Vec<Request>,
    /// What this frame owes the other zones.
    pub(super) shouts: Vec<Shout>,
}

impl Default for HistoryZone {
    fn default() -> Self {
        let mut zone = HistoryZone {
            blocks: BTreeMap::new(),
            asking: HashSet::new(),
            structure: Turns::default(),
            landed: 0,
            local: Local::default(),
            live: Live::default(),
            products: Products::default(),
            prep: Prefetch::default(),
            disp: Display::default(),
            todo_panel: TodoPanel::default(),
            todo: Vec::new(),
            todo_fold: TodoFold::default(),
            thinking: false,
            tool: false,
            placeholder: "thinking…".to_string(),
            fold: DEFAULT_FOLD,
            scroll_height: DEFAULT_SCROLL,
            cwd: String::new(),
            needs_layout: false,
            pending_focus: None,
            outbox: Vec::new(),
            shouts: Vec::new(),
        };
        zone.disp.set_line(DEFAULT_FOCUS_LINE);
        zone.products.set_margin(DEFAULT_MARGIN);
        zone
    }
}

impl HistoryZone {
    /// How far one wheel notch walks.
    fn walk(&self, shift: bool) -> usize {
        if shift {
            usize::from(self.disp.rows())
        } else {
            self.scroll_height.max(1)
        }
    }

    /// A line this front end has to say, on its own slot.
    pub fn notice(&mut self, text: String, level: Level, life: Life) {
        self.local.kill_short_lived();
        self.local.push_now(SlotKind::Notice { text, level, life });
    }

    /// Put a block under the sight line. The spot waits for the block's slot.
    pub fn focus(&mut self, turn: i64, block: i64) {
        match self.local.num_of(block) {
            Some(num) => self.focus_num(num),
            None => {
                self.pending_focus = Some((turn, block));
                self.ask_turn(turn);
            }
        }
    }

    /// `/new`: drop this conversation's section; the next attach asks for the new one.
    pub fn reset(&mut self) {
        self.destroy();
    }

    /// The local section goes, and the rows, the task list and the tail with it.
    pub(super) fn destroy(&mut self) {
        self.local.clear();
        self.products.clear();
        self.prep.rows.clear();
        self.blocks.clear();
        self.asking.clear();
        self.live.clear();
        self.todo.clear();
        self.todo_panel.changed();
        self.disp.follow();
        self.landed = 0;
        self.needs_layout = false;
    }

    /// Frames this zone asked for, taken by the host.
    pub(in crate::ui::tui::zone::main) fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.outbox)
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
        self.read_frame(frame);
        self.take_requests()
    }

    fn deliver_action(&mut self, action: &Action) -> bool {
        let handled = match action {
            Action::ScrollUp { shift } => {
                self.scroll(Dir::Older, self.walk(*shift));
                true
            }
            Action::ScrollDown { shift } => {
                self.scroll(Dir::Newer, self.walk(*shift));
                true
            }
            Action::Paste(_) => false,
            Action::Key(key) => match key.code {
                KeyCode::Char('t') if key.mods.ctrl => {
                    self.thinking = !self.thinking;
                    self.products.clear();
                    self.prep.rows.clear();
                    true
                }
                KeyCode::Char('o') if key.mods.ctrl => {
                    self.tool = !self.tool;
                    self.products.drop_fixed();
                    true
                }
                KeyCode::Up if key.mods.ctrl => {
                    self.step(Dir::Older);
                    true
                }
                KeyCode::Down if key.mods.ctrl => {
                    self.step(Dir::Newer);
                    true
                }
                KeyCode::PageUp if key.mods.ctrl => {
                    self.first();
                    true
                }
                KeyCode::PageDown if key.mods.ctrl => {
                    self.disp.follow();
                    true
                }
                KeyCode::PageUp => {
                    let rows = usize::from(self.disp.rows());
                    self.scroll(Dir::Older, rows);
                    true
                }
                KeyCode::PageDown => {
                    let rows = usize::from(self.disp.rows());
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
        self.disp.follow();
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
        self.apply_structure(snapshot);
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
                    self.live.echo(text);
                }
            }
            name::RETRACT => {
                self.live.retract();
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
        self.resize(rows);
        let width = usize::from(term.cols.max(1));
        if self.prep.width != width {
            self.refold(width);
        }
    }

    fn on_height_changed(&mut self, rows: u16) {
        self.resize(rows);
    }
}
