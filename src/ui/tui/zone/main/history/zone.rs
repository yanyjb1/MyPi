//! The history zone: what it claims, what it holds, and the contract it answers.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::focus::{Focus, Target};
use super::local::section::{Idx, Local};
use super::local::slot::{Level, Life, SlotKind};
use super::prefetch::{Range, Run, Start, walk};
use super::product::{Opts, Task, TodoPanel, esc_row, pin_rows};
use super::row::Row;
use super::{display, wire};
use crate::ui::tui::text::glyphs as g;

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

/// One running label: a call or a thought, and when it started.
pub(super) struct Pin {
    /// What the ESC row prints after the elapsed time.
    pub label: String,
    pub since: Instant,
    /// The call this label belongs to; `None` for a thought.
    pub call: Option<String>,
}

/// The chat history: what the reader sees of the conversation.
pub struct HistoryZone {
    /// The local section: render order and the content behind it.
    pub(super) local: Local,
    /// The laid segments around the screen.
    pub(super) run: Run,
    /// Where the screen starts.
    pub(super) range: Range,
    /// How many blocks may keep their content and product.
    pub(super) margin: usize,
    /// Positions whose content changed since the last draw.
    pub(super) dirty: BTreeSet<Idx>,
    /// Positions whose text grew in place: the piece the run lays again, and everything after it.
    pub(super) grown: BTreeMap<Idx, usize>,
    /// Blocks the last draw lacked; the next tick asks for their rounds.
    pub(super) missing: Vec<i64>,
    /// The checklist's own product.
    pub(super) todo_panel: TodoPanel,
    pub(super) todo: Vec<Task>,
    pub(super) todo_fold: TodoFold,
    /// What is still running, in the order it started; only the first one shows.
    pub(super) pin: Vec<Pin>,
    /// The phase the spinner reads.
    pub(super) tick: usize,
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
    /// A closed turn whose receipt does not line up: the ranges its own rows must come from.
    pub(super) pending: Option<Vec<[i64; 2]>>,
    /// A fresh attach lays the structure out again.
    pub(super) needs_layout: bool,
    /// A round is running: the statusline's busy glyph is on.
    pub(super) busy: bool,
    /// The place the window is waiting to point at, once its content is here.
    pub(super) pending_focus: Option<Target>,
    /// What this frame owes the session.
    pub(super) outbox: Vec<Request>,
    /// What this frame owes the other zones.
    pub(super) shouts: Vec<Shout>,
    /// Bumped whenever this zone's rows would differ.
    rev: u64,
}

impl Default for HistoryZone {
    fn default() -> Self {
        HistoryZone {
            local: Local::default(),
            run: Run::default(),
            range: Range::default(),
            margin: DEFAULT_MARGIN,
            dirty: BTreeSet::new(),
            grown: BTreeMap::new(),
            missing: Vec::new(),
            todo_panel: TodoPanel::default(),
            todo: Vec::new(),
            todo_fold: TodoFold::default(),
            pin: Vec::new(),
            tick: 0,
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
            pending: None,
            needs_layout: false,
            busy: false,
            pending_focus: None,
            outbox: Vec::new(),
            shouts: Vec::new(),
            rev: 0,
        }
    }
}

impl HistoryZone {
    /// Run one operation that needs the local section, the run and the range at once.
    pub(super) fn with_draw<R>(
        &mut self,
        f: impl FnOnce(&mut Local, &mut Run, &mut Range, &Opts<'_>) -> R,
    ) -> R {
        let HistoryZone {
            local,
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
        f(local, run, range, &opts)
    }

    /// The screen: the transcript, the checklist and the running label, in one run.
    pub fn render_rows(
        &mut self,
        term_w: u16,
        accent: Accent,
    ) -> Vec<ratatui::text::Line<'static>> {
        let width = usize::from(term_w.max(1));
        let height = usize::from(self.height).max(1);
        let pin = self.pin_rows(width);
        let spend = self.with_draw(|local, run, range, opts| {
            run.set_pin(pin);
            range.rows(run, local, opts, height)
        });
        let top = spend.top;
        self.with_draw(|_, run, _, _| run.trim(top, top + height));
        self.missing = spend.missing;
        display::lines(&spend.rows, accent)
    }

    /// One round's side effects, before the draw: the dirty set settles, and what the last draw
    /// lacked leaves as a request.
    pub(in crate::ui::tui::zone::main) fn tick(&mut self) {
        self.settle_dirty();
        let missing = std::mem::take(&mut self.missing);
        for block in missing {
            if let Some(turn) = self.turn_of(block) {
                self.ask_turn(turn);
            }
        }
    }

    /// Positions whose content changed lose their rows once; content past the budget leaves.
    pub(super) fn settle_dirty(&mut self) {
        let dirty = std::mem::take(&mut self.dirty);
        for idx in dirty {
            self.run.forget(idx);
            self.local.drop_product(idx);
        }
        let grown = std::mem::take(&mut self.grown);
        for (idx, from) in grown {
            self.run.forget_pieces(idx, from);
        }
        let HistoryZone {
            local,
            run,
            margin,
            asking,
            pending,
            ..
        } = self;
        let evicted = local.evict(*margin, |id, num| {
            run.holds(Idx::Fixed(num))
                || asking.iter().any(|[lo, hi]| (*lo..=*hi).contains(&id))
                || pending
                    .as_ref()
                    .is_some_and(|ranges| ranges.iter().any(|[lo, hi]| (*lo..=*hi).contains(&id)))
        });
        for num in evicted {
            run.forget(Idx::Fixed(num));
        }
    }

    /// The rows under the last segment: the checklist, then what is still running.
    fn pin_rows(&mut self, width: usize) -> Vec<Row> {
        let HistoryZone {
            todo_panel,
            todo,
            todo_fold,
            pin,
            tick,
            ..
        } = self;
        let checklist = todo_panel.rows(todo, *todo_fold, width).to_vec();
        let esc = pin.first().map(|running| {
            esc_row(
                g::STAR[*tick % g::STAR.len()],
                running.since.elapsed(),
                &running.label,
                running.call.is_none(),
            )
        });
        pin_rows(checklist, esc)
    }

    /// A thought started: its label leads until the thought ends.
    pub(super) fn thinking_started(&mut self) {
        if self.pin.iter().any(|pin| pin.call.is_none()) {
            return;
        }
        self.pin.push(Pin {
            label: "thinking".to_string(),
            since: Instant::now(),
            call: None,
        });
    }

    /// The thought is over.
    pub(super) fn thinking_ended(&mut self) {
        self.pin.retain(|pin| pin.call.is_some());
    }

    /// One call started; a running thought ends with it.
    pub(super) fn call_started(&mut self, call: &str, intent: Option<&str>) {
        self.thinking_ended();
        self.pin.retain(|pin| pin.call.as_deref() != Some(call));
        self.pin.push(Pin {
            label: intent.unwrap_or_default().to_string(),
            since: Instant::now(),
            call: Some(call.to_string()),
        });
    }

    /// One call ended: the next unfinished one leads.
    pub(super) fn call_ended(&mut self, call: &str) {
        self.pin.retain(|pin| pin.call.as_deref() != Some(call));
    }

    /// One position's text grew in place: the run lays its tail again, the product stays.
    pub(super) fn grow(&mut self, idx: Idx, from: usize) {
        self.grown
            .entry(idx)
            .and_modify(|at| *at = (*at).min(from))
            .or_insert(from);
    }

    /// One position's content changed: its rows are laid again at the next draw.
    pub(super) fn mark(&mut self, idx: Idx) {
        self.dirty.insert(idx);
        if let Some(card) = self.local.card_at(idx) {
            self.dirty.insert(card);
        }
    }

    /// One fixed position's content changed: its rows go.
    pub(super) fn forget(&mut self, num: i64) {
        self.mark(Idx::Fixed(num));
    }

    /// A line this front end has to say, in the mutable region.
    pub fn notice(&mut self, text: String, level: Level, life: Life) {
        for num in self.local.kill_short() {
            self.forget(num);
        }
        let idx = self.local.notice(text, level, life);
        self.mark(idx);
    }

    /// `/new`: drop this conversation's section; the next attach asks for the new one.
    pub fn reset(&mut self) {
        self.destroy();
    }

    /// The local section goes, and the run, the checklist and the range with it.
    pub(super) fn destroy(&mut self) {
        self.local.clear();
        self.run.clear();
        self.dirty.clear();
        self.grown.clear();
        self.missing.clear();
        self.range.follow();
        self.structure.clear();
        self.asking.clear();
        self.todo.clear();
        self.todo_panel.changed();
        self.pin.clear();
        self.tick = 0;
        self.pending_focus = None;
        self.pending = None;
        self.needs_layout = false;
        self.busy = false;
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
            Some(SlotKind::Block { .. })
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
        let missing = self.with_draw(|local, run, range, opts| {
            range.scroll(run, local, opts, height, older, rows)
        });
        for block in missing {
            if let Some(turn) = self.turn_of(block) {
                self.ask_turn(turn);
            }
        }
    }
}

impl HistoryZone {
    fn bump(&mut self) {
        self.rev = self.rev.wrapping_add(1);
    }

    /// How often the ESC row changes on its own: the starburst turns 30 times a second.
    pub(crate) fn anim(&self) -> Option<Duration> {
        (!self.pin.is_empty()).then(|| Duration::from_millis(1000 / 30))
    }
}

impl SubZone for HistoryZone {
    fn claims(&self) -> &'static [Claim] {
        CLAIMS
    }

    fn kinds(&self) -> &'static [Kind] {
        KINDS
    }

    fn revision(&self) -> u64 {
        self.rev
    }

    fn deliver_event(&mut self, frame: &Frame, _size: TermSize) -> Vec<Request> {
        wire::read_frame(self, frame);
        let out = self.take_requests();
        self.bump();
        out
    }

    fn deliver_action(&mut self, action: &Action) -> bool {
        // Keys read the layout: whatever the stream changed is laid before they run.
        self.settle_dirty();
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
                    self.local.drop_products();
                    self.run.clear();
                    true
                }
                KeyCode::Char('o') if key.mods.ctrl => {
                    self.tool = !self.tool;
                    self.local.drop_products();
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
        self.bump();
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
        self.bump();
    }

    fn detach(&mut self) {
        self.destroy();
        self.bump();
    }

    fn structure(&mut self, snapshot: &Turns) {
        wire::apply_structure(self, snapshot);
        self.bump();
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
                    // Cards name directories: the paths they drew are stale now.
                    self.local.drop_products();
                    self.run.clear();
                }
            }
            name::ECHO => {
                if let Some(text) = content.get("text").and_then(Value::as_str) {
                    let at = self.local.running_mut().echo(text);
                    self.mark(Idx::Turn(at as u32));
                    self.shouts
                        .push(Shout::new(name::TICK, serde_json::json!({})));
                }
            }
            name::RETRACT => {
                if let Some(at) = self.local.running_mut().retract() {
                    self.mark(Idx::Turn(at as u32));
                }
                let reason = content
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("the message never opened a round");
                self.notice(reason.to_string(), Level::Warn, Life::Short);
            }
            name::NOTICE => {
                let level = match content.get("level").and_then(Value::as_str) {
                    Some("error") => Level::Error,
                    Some("warn") => Level::Warn,
                    _ => Level::Info,
                };
                let text = content
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                self.notice(text, level, Life::Short);
            }
            _ => {}
        }
        self.bump();
    }
}

impl Pane for HistoryZone {
    fn assign(&mut self, term: TermSize, rows: u16) {
        let height_moved = self.height != rows;
        self.height = rows;
        let width = usize::from(term.cols.max(1));
        let width_moved = self.width != width;
        if width_moved {
            self.width = width;
            self.run.set_width(width);
            self.local.drop_products();
        }
        if height_moved || width_moved {
            self.bump();
        }
    }

    fn on_height_changed(&mut self, rows: u16) {
        if self.height != rows {
            self.height = rows;
            self.bump();
        }
    }
}
