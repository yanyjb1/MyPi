//! The history zone: what it claims, what it asks for, and one row pass per frame.

use std::collections::{BTreeMap, HashSet};

use ratatui::text::Line;
use serde_json::{Value, json};

use super::cache::BlockCache;
use super::live::Live;
use super::local::{Level, Life, Local, SlotKind};
use super::render::cards::chat::live_rows;
use super::render::cards::todo::{self as todo_rows, Task};
use super::render::notice::notice_block;
use super::render::{self, Opts};
use super::shape::{Block, Kind as BlockKind};
use super::view::{Dir, Viewport};

use crate::ui::tui::TodoFold;
use crate::ui::tui::action::{Action, Key, KeyCode, Mods};
use crate::ui::tui::theme::{HistoryTheme, Token};
use crate::ui::tui::zone::{Claim, Kind, Request, Shout, Structure, SubZone, TermSize, name};

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

/// Rows between the sight line and the top of the zone.
const FOCUS_LINE: u16 = 3;

/// Blocks on each side of the sight line that keep their rows.
const MARGIN: i64 = 32;

/// The chat history: what the reader sees of the conversation.
pub struct HistoryZone {
    /// Every landed block this front end holds, by id.
    blocks: BTreeMap<i64, Block>,
    /// Ranges already asked for, so one turn is not asked twice.
    asking: HashSet<[i64; 2]>,
    /// The structural window, as last handed down.
    structure: Structure,
    /// The latest block id that has taken a slot.
    landed: i64,
    /// What the conversation is made of, in order.
    local: Local,
    /// The tail still growing.
    live: Live,
    view: Viewport,
    cache: BlockCache,
    /// The model's checklist, as the session replays it.
    todo: Vec<Task>,
    todo_fold: TodoFold,
    /// The thinking chain's own copy.
    thinking: bool,
    /// Tool exchanges' own copy.
    tool: bool,
    placeholder: String,
    /// Where the tools ran, for shortening paths.
    cwd: String,
    /// Rows the main zone handed down.
    rows: u16,
    /// The last context size shouted, so one number is not repeated every frame.
    usage: u64,
    /// A fresh attach lays the structure out again.
    needs_layout: bool,
    /// A spot to put the sight line on, once its block has a slot.
    pending_focus: Option<(i64, i64)>,
    /// The next attach is spared the destroy: a new conversation owns what is there.
    reprieve: bool,
    /// What this frame owes the session.
    outbox: Vec<Request>,
    /// What this frame owes the other zones.
    shouts: Vec<Shout>,
}

impl Default for HistoryZone {
    fn default() -> Self {
        let mut zone = Self {
            blocks: BTreeMap::new(),
            asking: HashSet::new(),
            structure: Structure::default(),
            landed: 0,
            local: Local::default(),
            live: Live::default(),
            view: Viewport::default(),
            cache: BlockCache::default(),
            todo: Vec::new(),
            todo_fold: TodoFold::default(),
            thinking: false,
            tool: false,
            placeholder: "thinking…".to_string(),
            cwd: String::new(),
            rows: 0,
            usage: 0,
            needs_layout: false,
            pending_focus: None,
            reprieve: true,
            outbox: Vec::new(),
            shouts: Vec::new(),
        };
        zone.view.set_line(FOCUS_LINE);
        zone.cache.set_budget(MARGIN.max(1) as usize * 2 + 1);
        zone
    }
}

impl HistoryZone {
    /// A line this front end has to say, on its own slot.
    pub fn notice(&mut self, text: String, level: Level, life: Life) {
        self.local.kill_short_lived();
        self.local.push_now(SlotKind::Notice { text, level, life });
    }

    /// Put a block under the sight line. The spot is kept until the block has a slot,
    /// because the tree leaves it in the memo before this zone has the structure.
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
        self.reprieve = true;
        self.destroy();
    }

    /// The local section goes, and the rows, the task list and the tail with it.
    fn destroy(&mut self) {
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

    /// The rows of one frame: the conversation, then the pinned rows.
    pub fn render_rows(&mut self, term_w: u16, t: &HistoryTheme) -> Vec<Line<'static>> {
        let width = usize::from(term_w.max(1));
        self.cache
            .sync(width, &self.cwd, (self.thinking, self.tool));
        let opts = Opts {
            thinking: self.thinking,
            tool: self.tool,
            placeholder: &self.placeholder,
            cwd: &self.cwd,
            rows: usize::from(self.rows),
        };

        let pinned = todo_rows::rows(
            &self.todo,
            self.todo_fold,
            width,
            t,
            usize::from(self.rows) / 2,
        );
        let tail = if self.live.is_empty() {
            Vec::new()
        } else {
            live_rows(self.live.blocks(), t, &opts, width)
        };

        let local = &self.local;
        let blocks = &self.blocks;
        let cache = &mut self.cache;
        let view = &mut self.view;
        view.compose(
            local,
            |num| {
                let Some(slot) = local.at(num) else {
                    return Vec::new();
                };
                match &slot.kind {
                    SlotKind::Next => Vec::new(),
                    SlotKind::Notice { text, level, .. } => notice_block(text, *level, t),
                    SlotKind::Block(id) => match blocks.get(id) {
                        Some(block) => {
                            let paired = blocks.get(&(id + 1)).filter(|next| block.answers(next));
                            cache
                                .paint(num, || render::block(block, paired, t, &opts, width))
                                .to_vec()
                        }
                        None => vec![Line::from(t.fg("…", Token::SystemText))],
                    },
                }
            },
            &tail,
            &pinned,
        )
    }

    /// Take what this frame owes the session.
    fn take_out(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.outbox)
    }

    /// Ask for one turn's blocks, unless they are here or already asked for.
    fn ask_turn(&mut self, turn: i64) {
        let Some(held) = self.structure.turns().iter().find(|t| t.id == turn) else {
            return;
        };
        for range in &held.ranges {
            if self.asking.contains(range) {
                continue;
            }
            let [lo, hi] = *range;
            let have = (lo..=hi).all(|id| self.blocks.contains_key(&id));
            if have {
                continue;
            }
            self.asking.insert(*range);
            self.outbox
                .push(Request::call("blocks", json!({"ranges": [[lo, hi]]})));
        }
    }

    /// The turn a block belongs to.
    fn turn_of(&self, block: i64) -> Option<i64> {
        self.structure
            .turns()
            .iter()
            .find(|turn| turn.owns(block))
            .map(|turn| turn.id)
    }

    /// Keep the anchor's own turn and its neighbours in hand.
    fn ensure_around(&mut self) {
        let anchor = self.view.anchor();
        let Some(block) = self.local.at(anchor).and_then(|slot| match slot.kind {
            SlotKind::Block(id) => Some(id),
            _ => None,
        }) else {
            return;
        };
        let wanted: Vec<i64> = {
            let turns = self.structure.turns();
            let Some(at) = turns.iter().position(|turn| turn.owns(block)) else {
                return;
            };
            [-1i64, 0, 1]
                .iter()
                .filter_map(|offset| {
                    let index = at as i64 + offset;
                    (index >= 0 && (index as usize) < turns.len()).then(|| turns[index as usize].id)
                })
                .collect()
        };
        for turn in wanted {
            self.ask_turn(turn);
        }
    }

    /// Decode a `blocks` array, keep the rows, and give the newest ones their numbers.
    fn take_blocks(&mut self, rows: &Value) {
        let Some(items) = rows.as_array() else {
            return;
        };
        for row in items {
            let Some(block) = Block::of(row) else {
                continue;
            };
            let id = block.id;
            if block.usage.prompt > 0 {
                self.usage = block.usage.context();
            }
            let fresh = id > self.landed;
            self.blocks.insert(id, block);
            if !fresh {
                continue;
            }
            self.landed = id;
            if self.needs_layout {
                // The attach's own blocks belong to the past; the layout pass numbers them.
                continue;
            }
            let num = self.local.push_now(SlotKind::Block(id));
            self.cache.drop_num(num);
        }
    }

    /// Lay the whole structure out as the past, newest block at -1.
    fn lay_out(&mut self) {
        let turns = self.structure.turns();
        if turns.is_empty() {
            return;
        }
        let mut ids: Vec<i64> = Vec::new();
        for turn in turns.iter().rev() {
            for [lo, hi] in &turn.ranges {
                ids.extend(*lo..=*hi);
            }
        }
        self.local.clear();
        for id in ids.iter().rev() {
            self.local.push_older(SlotKind::Block(*id));
        }
        self.landed = ids.last().copied().unwrap_or(0);
    }

    /// Read one frame's content: a live piece, a closed round, a notice, a change.
    fn read_stream(&mut self, packet: &crate::ui::tui::zone::Packet) {
        let content = &packet.content;
        match packet.name.as_str() {
            "notice" => {
                let level = match content.get("level").and_then(Value::as_str) {
                    Some("error") => Level::Error,
                    Some("warn") => Level::Warn,
                    _ => Level::Info,
                };
                let text = content
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let life = if level == Level::Error {
                    Life::Long
                } else {
                    Life::Short
                };
                self.notice(text, level, life);
            }
            "change" => {
                if content.get("action").and_then(Value::as_str) == Some("rewind") {
                    // Another front end rewound the session: the structure is the truth, so
                    // ask for it again and let the slots follow.
                    self.outbox.push(Request::Structure);
                }
            }
            _ => {
                if content.get("ranges").is_some() {
                    self.closed(content);
                    return;
                }
                self.piece(content);
            }
        }
    }

    /// One live piece from the running turn.
    fn piece(&mut self, content: &Value) {
        let kind = content
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let text = content
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let call = content
            .get("call_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match kind {
            "text" => self.live.text(text),
            "thinking" => self.live.thinking(text),
            "arguments" => self.live.arguments(call, text),
            "boundary" => self.live.boundary(),
            "tool_start" => {
                let name = content
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let args = content
                    .get("args")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let intent = content.get("intent").and_then(Value::as_str);
                self.live.tool_start(call, name, args, intent);
            }
            "tool_progress" => self.live.tool_progress(call, text),
            "tool_end" => {
                let ok = content.get("ok").and_then(Value::as_bool).unwrap_or(false);
                let result = content
                    .get("result")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let memo = content.get("memo").and_then(Value::as_str);
                let took = content
                    .get("duration_ms")
                    .and_then(Value::as_i64)
                    .unwrap_or(0);
                self.live.tool_end(call, ok, result, memo, took);
            }
            _ => {}
        }
        self.shouts
            .push(Shout::new(name::BUSY, json!({"on": true})));
    }

    /// A round closed: its ranges are in hand, the tail is not needed any more.
    fn closed(&mut self, content: &Value) {
        self.live.clear();
        self.shouts
            .push(Shout::new(name::BUSY, json!({"on": false})));
        let Some(ranges) = content.get("ranges").and_then(ranges_of) else {
            return;
        };
        for range in ranges {
            let [lo, hi] = range;
            let have = (lo..=hi).all(|id| self.blocks.contains_key(&id));
            if have {
                continue;
            }
            self.outbox
                .push(Request::call("blocks", json!({"ranges": [[lo, hi]]})));
        }
        self.outbox.push(Request::Structure);
    }

    /// The slots whose blocks the session no longer owns are gone with them. This is how
    /// a rewind reaches this zone: the tree holds the reply's `cut`, the structure is the
    /// truth, and the tail is simply what the structure stopped owning.
    fn drop_unowned(&mut self) {
        let keep: Vec<i64> = self
            .local
            .iter()
            .filter_map(|slot| match slot.kind {
                SlotKind::Block(id) if !self.structure.owns(id) => Some(slot.num),
                _ => None,
            })
            .collect();
        let cut = keep
            .first()
            .copied()
            .unwrap_or(self.local.newest().unwrap_or(0) + 1);
        if cut >= 0 {
            self.local.truncate_now(cut as usize);
        }
        self.blocks.retain(|id, _| self.structure.owns(*id));
        self.landed = self.blocks.keys().next_back().copied().unwrap_or(0);
        let retyped = self
            .blocks
            .values()
            .next_back()
            .filter(|block| block.kind == BlockKind::User)
            .and_then(|block| block.content.clone().or_else(|| block.text.clone()));
        if let Some(text) = retyped {
            self.shouts
                .push(Shout::new(name::PREFILL, json!({"text": text})));
        }
    }

    /// The checklist, as one flat list.
    fn take_todo(&mut self, value: &Value) {
        if value.is_null() {
            self.todo.clear();
            return;
        }
        let Some(items) = value.as_array() else {
            return;
        };
        self.todo = items
            .iter()
            .filter_map(|item| {
                Some(Task {
                    text: item.get("text")?.as_str()?.to_string(),
                    done: item.get("done").and_then(Value::as_bool).unwrap_or(false),
                })
            })
            .collect();
    }

    /// A screen older or newer / one slot older or newer.
    fn scroll(&mut self, dir: Dir, page: bool) {
        if page {
            let heights = &self.cache;
            let mut view = std::mem::take(&mut self.view);
            view.page(&self.local, |num| heights.known(num).unwrap_or(1), dir);
            self.view = view;
        } else {
            self.view.step(&self.local, dir);
        }
        self.ensure_around();
    }

    fn toggle_thinking(&mut self) {
        self.thinking = !self.thinking;
        self.cache.clear();
    }

    fn toggle_tool(&mut self) {
        self.tool = !self.tool;
        self.cache.clear();
    }
}

impl SubZone for HistoryZone {
    fn claims(&self) -> &'static [Claim] {
        CLAIMS
    }

    fn kinds(&self) -> &'static [Kind] {
        KINDS
    }

    fn deliver_event(
        &mut self,
        packet: &crate::ui::tui::zone::Packet,
        _size: TermSize,
    ) -> Vec<Request> {
        match (packet.kind.as_str(), packet.name.as_str()) {
            ("stream", _) => self.read_stream(packet),
            ("call", "init") => {
                if let Some(rows) = packet.content.get("blocks") {
                    self.take_blocks(rows);
                }
                if let Some(todo) = packet.content.get("todo") {
                    self.take_todo(todo);
                }
                self.outbox.push(Request::Structure);
            }
            ("call", "blocks") => {
                self.asking.clear();
                if let Some(rows) = packet.content.get("blocks") {
                    self.take_blocks(rows);
                }
            }
            ("call", "todo") => {
                if let Some(todo) = packet.content.get("todo") {
                    self.take_todo(todo);
                }
            }
            _ => {}
        }
        self.take_out()
    }

    fn deliver_action(&mut self, a: &Action) -> bool {
        match a {
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
                    self.toggle_thinking();
                    true
                }
                KeyCode::Char('o') if key.mods.ctrl => {
                    self.toggle_tool();
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
        if self.reprieve {
            self.reprieve = false;
        } else {
            self.destroy();
        }
        self.needs_layout = true;
        self.view.follow();
    }

    fn detach(&mut self) {
        self.destroy();
    }

    fn structure(&mut self, snapshot: &Structure) {
        self.structure = snapshot.clone();
        if self.needs_layout && !self.structure.turns().is_empty() {
            self.needs_layout = false;
            self.lay_out();
        } else {
            self.drop_unowned();
        }
        if let Some((_, block)) = self.pending_focus
            && let Some(num) = self.local.num_of(block)
        {
            self.pending_focus = None;
            self.view.focus_num(num);
        }
        self.ensure_around();
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
        if event == name::CWD
            && let Some(path) = content.get("path").and_then(Value::as_str)
            && self.cwd != path
        {
            self.cwd.clear();
            self.cwd.push_str(path);
        }
    }
}

impl HistoryZone {
    /// A new row budget: the seam stays, the row inside it is clamped.
    fn resize(&mut self, rows: u16) {
        self.rows = rows;
        self.view.assign(rows);
        let cache = &self.cache;
        self.view
            .on_resize(&self.local, |num| cache.known(num).unwrap_or(1));
    }

    /// Align the sight line's round's first or last block.
    fn align(&mut self, last: bool) {
        let anchor = self.view.anchor();
        let Some(id) = self.local.at(anchor).and_then(|slot| match slot.kind {
            SlotKind::Block(id) => Some(id),
            _ => None,
        }) else {
            return;
        };
        let Some(turn) = self.turn_of(id) else { return };
        let nums: Vec<i64> = self
            .local
            .iter()
            .filter(|slot| match slot.kind {
                SlotKind::Block(id) => self.turn_of(id) == Some(turn),
                _ => false,
            })
            .map(|slot| slot.num)
            .collect();
        let num = if last {
            nums.iter().max()
        } else {
            nums.iter().min()
        };
        if let Some(num) = num {
            self.view.focus_num(*num);
            self.ensure_around();
        }
    }
}

/// A frame's ranges: `[[lo, hi], …]`.
fn ranges_of(value: &Value) -> Option<Vec<[i64; 2]>> {
    let mut out = Vec::new();
    for item in value.as_array()? {
        let pair = item.as_array()?;
        if pair.len() != 2 {
            return None;
        }
        out.push([pair[0].as_i64()?, pair[1].as_i64()?]);
    }
    Some(out)
}
