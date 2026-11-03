//! The history zone: what it claims, what it holds, and the contract it answers.

use std::collections::{BTreeMap, HashSet};

use serde_json::Value;

use super::cache::Cache;
use super::live::Live;
use super::local::{Level, Life, Local, SlotKind};
use super::paint::todo::Task;
use super::shape::Block;
use super::view::{Dir, Viewport};

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

/// Rendered blocks kept in the cache until the config answers.
pub(super) const DEFAULT_MARGIN: usize = 64;

/// Rows a card head or tail may take until the config answers.
pub(super) const DEFAULT_FOLD: usize = 10;

/// The config keys this zone asks for once per attachment.
const CONFIG_KEYS: &[&str] = &[
    "config.ui.tui.history.focusLine",
    "config.ui.tui.history.cardFoldBudget",
    "config.ui.tui.renderMargin",
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
    pub(super) view: Viewport,
    pub(super) cache: Cache,
    /// The model's checklist, as the session replays it.
    pub(super) todo: Vec<Task>,
    pub(super) todo_fold: TodoFold,
    /// The thinking chain's own copy.
    pub(super) thinking: bool,
    /// Tool exchanges' own copy.
    pub(super) tool: bool,
    pub(super) placeholder: String,
    /// Rows between the sight line and the top of the zone.
    pub(super) focus_line: u16,
    /// Rows a card head or a card tail may take before it folds.
    pub(super) fold: usize,
    /// Where the tools ran, for shortening paths.
    pub(super) cwd: String,
    /// Rows the main zone handed down.
    pub(super) rows: u16,
    /// The last context size shouted, so one number is not repeated every frame.
    pub(super) usage: u64,
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
            view: Viewport::default(),
            cache: Cache::default(),
            todo: Vec::new(),
            todo_fold: TodoFold::default(),
            thinking: false,
            tool: false,
            placeholder: "thinking…".to_string(),
            focus_line: DEFAULT_FOCUS_LINE,
            fold: DEFAULT_FOLD,
            cwd: String::new(),
            rows: 0,
            usage: 0,
            needs_layout: false,
            pending_focus: None,
            outbox: Vec::new(),
            shouts: Vec::new(),
        };
        zone.view.set_line(DEFAULT_FOCUS_LINE);
        zone.cache.set_budget(DEFAULT_MARGIN);
        zone
    }
}

impl HistoryZone {
    /// A line this front end has to say, on its own slot.
    pub fn notice(&mut self, text: String, level: Level, life: Life) {
        self.local.kill_short_lived();
        self.local.push_now(SlotKind::Notice { text, level, life });
    }

    /// Put a block under the sight line. The spot waits for the block's slot.
    pub fn focus(&mut self, turn: i64, block: i64) {
        match self.local.num_of(block) {
            Some(num) => self.view.focus_num(num),
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
        self.cache.clear();
        self.blocks.clear();
        self.asking.clear();
        self.live.clear();
        self.todo.clear();
        self.view.follow();
        self.landed = 0;
        self.usage = 0;
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
        match action {
            Action::ScrollUp { shift } => {
                self.scroll(Dir::Older, *shift);
                true
            }
            Action::ScrollDown { shift } => {
                self.scroll(Dir::Newer, *shift);
                true
            }
            Action::Paste(_) => false,
            Action::Key(key) => match key.code {
                KeyCode::Char('t') if key.mods.ctrl => {
                    self.thinking = !self.thinking;
                    self.cache.clear();
                    true
                }
                KeyCode::Char('o') if key.mods.ctrl => {
                    self.tool = !self.tool;
                    self.cache.clear();
                    true
                }
                KeyCode::Up if key.mods.ctrl => {
                    self.view.step(&self.local, Dir::Older);
                    self.ensure_around();
                    true
                }
                KeyCode::Down if key.mods.ctrl => {
                    self.view.step(&self.local, Dir::Newer);
                    self.ensure_around();
                    true
                }
                KeyCode::PageUp if key.mods.ctrl => {
                    self.view.first(&self.local);
                    self.ensure_around();
                    true
                }
                KeyCode::PageDown if key.mods.ctrl => {
                    self.view.follow();
                    true
                }
                KeyCode::PageUp => {
                    self.scroll(Dir::Older, true);
                    true
                }
                KeyCode::PageDown => {
                    self.scroll(Dir::Newer, true);
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
        }
    }

    fn attach(&mut self, _session: i64, _size: TermSize) {
        self.destroy();
        self.needs_layout = true;
        self.view.follow();
        for key in CONFIG_KEYS {
            self.outbox
                .push(Request::call("config", serde_json::json!({"key": key})));
        }
    }

    fn detach(&mut self) {
        self.destroy();
    }

    fn structure(&mut self, snapshot: &Turns) {
        self.apply_structure(snapshot);
    }

    fn request_height(&self, _term: TermSize) -> u16 {
        u16::MAX
    }

    fn assign(&mut self, _term: TermSize, rows: u16) {
        self.resize(rows);
    }

    fn on_height_changed(&mut self, rows: u16) -> bool {
        self.resize(rows);
        true
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
