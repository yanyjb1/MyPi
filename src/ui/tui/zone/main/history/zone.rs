//! The history zone itself: what it reads from config, the keys it claims, and one row
//! pass per frame.

use std::time::Duration;

use ratatui::text::Line;

use super::ask::Ask;
use super::cache::{BlockCache, Item};
use super::layout;
use super::local::{Local, SlotKind};
use super::render::cards::{chat, todo as todo_rows};
use super::render::theme::HistoryTheme;
use super::resize;
use super::todo::TaskList;
use super::view::{Dir, Viewport};

use crate::server::config::read::config::TodoFold;
use crate::server::storage::BlockKind;
use crate::ui::session::{Level, Life, View};
use crate::ui::tui::action::{Action, Key, KeyCode, Mods};
use crate::ui::tui::zone::{Claim, Ctx, Event, Request, SubZone, TermSize};

/// Everything the zone reads from config, handed over once at assembly.
#[derive(Debug, Clone)]
pub struct Config {
    /// Rows between the sight line and the top of the zone.
    pub focus_line: u16,
    /// How many blocks on each side of the sight line keep their rows.
    pub focus_margin: usize,
    /// How long the reader must be still before the receipt goes out.
    pub rest_ms: u64,
    /// Blocks already laid out before more is asked for.
    pub overscan: usize,
    /// What a folded thinking chain shows.
    pub on_thinking: String,
    /// Ctrl+T's starting position.
    pub unfold_thinking: bool,
    /// Ctrl+O's starting position.
    pub unfold_tool: bool,
    /// How the task list folds.
    pub todo_fold: TodoFold,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            focus_line: 3,
            focus_margin: 32,
            rest_ms: 300,
            overscan: 12,
            on_thinking: "thinking…".to_string(),
            unfold_thinking: false,
            unfold_tool: false,
            todo_fold: TodoFold::default(),
        }
    }
}

/// The keys the history zone consumes. The input zone yields Ctrl+arrows to it.
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

/// The chat history: what the reader sees of the conversation.
pub struct HistoryZone {
    local: Local,
    view: Viewport,
    cache: BlockCache,
    /// Blocks already laid out before more is asked for.
    overscan: usize,
    /// How many blocks around the sight line keep their rows.
    margin: i64,
    /// How long the reader must be still before the receipt goes out.
    silence: Duration,
    /// The task list: the server's start, then one increment per call.
    task: TaskList,
    todo_fold: TodoFold,
    /// What is owed to the session, collected this frame.
    ask: Vec<Ask>,
    /// The first attach is spared the destroy: a new conversation owns what is there.
    reprieve: bool,
}

impl Default for HistoryZone {
    fn default() -> Self {
        Self {
            local: Local::default(),
            view: Viewport::default(),
            cache: BlockCache::new(),
            overscan: 12,
            margin: 32,
            silence: Duration::from_millis(300),
            task: TaskList::default(),
            todo_fold: TodoFold::default(),
            ask: Vec::new(),
            reprieve: true,
        }
    }
}

impl HistoryZone {
    /// Take the config, once.
    pub fn set_config(&mut self, cfg: &Config) {
        self.view.set_line(cfg.focus_line);
        self.cache.set_budget(cfg.focus_margin.max(1) * 2 + 1);
        self.cache.set_placeholder(&cfg.on_thinking);
        self.cache
            .set_switches(cfg.unfold_thinking, cfg.unfold_tool);
        self.overscan = cfg.overscan.max(1);
        self.margin = cfg.focus_margin.max(1) as i64;
        self.silence = Duration::from_millis(cfg.rest_ms);
        self.todo_fold = cfg.todo_fold;
    }

    /// Warm the languages the transcript actually uses, newest first.
    pub fn warm(&self, view: &View<'_>) {
        let names: Vec<String> = view
            .chain()
            .filter_map(|b| b.name().map(|n| n.to_string()))
            .collect();
        super::render::text::highlight::warm_in_order(names.iter().map(String::as_str));
    }

    /// Take what this frame owes the session.
    pub fn take_out(&mut self) -> Vec<Ask> {
        std::mem::take(&mut self.ask)
    }

    /// A line this front end has to say, on its own slot.
    pub fn notice(&mut self, text: String, level: Level, life: Life) {
        if life == Life::Short {
            self.local.kill_short_lived();
        }
        self.local.append(SlotKind::Notice { text, level, life });
    }

    /// Open a conversation: destroy the old section, unless the first attach spares it. The
    /// first attach still asks for the structure: a new conversation's answer is empty.
    fn open(&mut self, session: i64) {
        if self.reprieve {
            self.reprieve = false;
        } else {
            self.destroy();
        }
        self.ask.push(Ask::Init { session });
    }

    /// `/new`: drop this conversation's section; the next attach asks for the new one.
    pub fn reset(&mut self) {
        self.reprieve = true;
        self.destroy();
    }

    /// Put a landed block under the sight line; an absent block asks for its round.
    pub fn focus(&mut self, turn: i64, block: i64) {
        match self.local.num_of(block) {
            Some(num) => self.view.focus_num(num),
            None => self.ask.push(Ask::Jump { turn }),
        }
    }

    /// The local section goes, and the rows, the task list and the thinking state with it.
    fn destroy(&mut self) {
        self.local.clear();
        self.cache.clear();
        self.task.clear();
        self.ask.clear();
        self.view.follow();
    }

    /// The rows of one frame: the conversation, then the pinned rows.
    pub fn render_rows(
        &mut self,
        view: &View<'_>,
        term_w: u16,
        t: &HistoryTheme,
    ) -> Vec<Line<'static>> {
        let width = usize::from(term_w.max(1));
        let mut items = self.items(view);
        self.cache.sync(
            &items,
            view.generation,
            view.tail.version(),
            width,
            view.cwd,
        );
        self.cache.set_busy(view.busy);
        if self.land(view) {
            items = self.items(view);
            self.trim(view, &items);
        }
        self.todo(view);

        let plan = resize::plan(
            &self.local,
            self.view.anchor(),
            self.cache.cached_numbers(),
            self.margin,
        );
        layout::visible::apply(&mut self.cache, &items, &plan.visible, t);

        let mut pinned = todo_rows::rows(
            self.task.phases(),
            self.task.touched(),
            self.todo_fold,
            width,
            t,
            usize::from(self.view.rows()) / 2,
        );
        let skip = self.thinking_slot(&items);
        pinned.extend(self.thinking_rows(&items, t));

        let tail = view
            .tail
            .tool_output()
            .map(|o| chat::tool_output_rows(&o.text, t, width))
            .unwrap_or_default();

        self.view.rows(
            &self.local,
            &items,
            &mut self.cache,
            &pinned,
            &tail,
            skip,
            t,
            width,
        )
    }

    /// The blocks of this frame, in order, with tool exchanges glued to one node.
    fn items<'a>(&mut self, view: &'a View<'_>) -> Vec<Item<'a>> {
        let blocks: Vec<crate::ui::session::BlockRef<'a>> = view.chain().collect();
        let waiting = self.waiting();
        if waiting.len() < blocks.iter().filter(|b| b.id().is_none()).count() {
            let need = blocks.iter().filter(|b| b.id().is_none()).count() - waiting.len();
            for _ in 0..need {
                self.local.append(SlotKind::Empty);
            }
        }
        let waiting = self.waiting();
        let mut out = Vec::with_capacity(blocks.len());
        let mut live = 0usize;
        let mut at = 0usize;
        while at < blocks.len() {
            let block = blocks[at];
            let paired = super::render::block::pair(&blocks, at);
            let num = match block.id().and_then(|id| self.local.num_of(id)) {
                Some(num) => num,
                None => {
                    let num = waiting.get(live).copied().unwrap_or_default();
                    live += 1;
                    num
                }
            };
            let version = if block.is_live() {
                view.tail.version()
            } else {
                0
            };
            out.push(Item {
                num,
                block,
                paired,
                version,
            });
            at += if paired.is_some() { 2 } else { 1 };
        }
        out
    }

    /// The numbers of the slots still waiting for a block id, oldest first.
    fn waiting(&self) -> Vec<i64> {
        self.local
            .iter()
            .filter(|s| matches!(s.kind, SlotKind::Empty))
            .map(|s| s.num)
            .collect()
    }

    /// Land the ids the session handed out, and let dead notices go. The landed numbers keep
    /// their places; their rows are dropped so the next pass paints the landed version.
    fn land(&mut self, view: &View<'_>) -> bool {
        let waiting = self.waiting();
        if waiting.is_empty() {
            return false;
        }
        let ids: Vec<i64> = view
            .blocks
            .iter()
            .rev()
            .take(waiting.len())
            .map(|b| b.block.id)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        self.local.land(&ids);
        for num in &waiting {
            self.cache.invalidate(*num);
        }
        self.local.kill_short_lived();
        true
    }

    /// Drop the rows outside the circle, once, at landing.
    ///
    /// One circle when the reader is on the part that just landed: the sight line's block plus
    /// `margin` either way. Two circles when the reader is elsewhere: that one, plus the bottom
    /// of the mutable region and `margin` above it. Two passes: oldest write time first, skipping
    /// the part the reader is looking at; then whatever is left goes by distance from the edge.
    fn trim(&mut self, view: &View<'_>, items: &[Item<'_>]) {
        if view.busy {
            return;
        }
        let anchor = self.view.anchor();
        let margin = self.margin;
        let landing = items
            .iter()
            .rev()
            .find_map(|it| it.block.id().and_then(|id| view.turn(id)));
        let reader = self.round_at(items, view, anchor);
        let two_places = reader != landing;
        let bottom = self.local.newest().unwrap_or(anchor);
        let mut outside: Vec<i64> = self
            .cache
            .held_numbers()
            .into_iter()
            .filter(|num| {
                let near_anchor = (*num - anchor).abs() <= margin;
                let near_bottom = two_places && *num <= bottom && bottom - *num <= margin;
                !near_anchor && !near_bottom
            })
            .collect();
        outside.sort_by_key(|num| self.cache.written(*num).unwrap_or(0));
        let mut kept_back: Vec<i64> = Vec::new();
        for num in outside {
            if reader.is_some() && self.round_at(items, view, num) == reader {
                kept_back.push(num);
                continue;
            }
            self.cache.drop_slot(num);
        }
        kept_back.sort_by_key(|num| std::cmp::Reverse((num - anchor).abs()));
        for num in kept_back {
            self.cache.drop_slot(num);
        }
    }

    /// The round a slot belongs to.
    fn round_at(&self, items: &[Item<'_>], view: &View<'_>, num: i64) -> Option<i64> {
        Self::block_at(items, num).and_then(|id| view.turn(id))
    }

    /// The block id on one slot, if it has one. The mutable region has none.
    fn block_at(items: &[Item<'_>], num: i64) -> Option<i64> {
        items
            .iter()
            .find(|it| it.num == num)
            .and_then(|it| it.block.id())
    }

    /// Keep the task list: the server's start, then the calls whose result has landed.
    fn todo(&mut self, view: &View<'_>) {
        self.task.sync(view);
    }

    /// The thinking chain's rows: pinned at the bottom, above nothing.
    fn thinking_rows(&mut self, items: &[Item<'_>], t: &HistoryTheme) -> Vec<Line<'static>> {
        match self.thinking_slot(items) {
            Some(num) => match items.iter().find(|it| it.num == num) {
                Some(item) => self.cache.rows(item, t),
                None => Vec::new(),
            },
            None => Vec::new(),
        }
    }

    /// The slot the thinking chain sits on: drawn pinned, so the walk skips it.
    fn thinking_slot(&self, items: &[Item<'_>]) -> Option<i64> {
        items
            .iter()
            .rev()
            .find(|it| it.block.kind() == BlockKind::Reasoning)
            .map(|it| it.num)
    }

    /// Ask for more when the direction the reader moved in is running out of rows.
    fn ask_more(&mut self, items: &[Item<'_>], dir: Dir) {
        let ahead = self.view.overscan(&self.local, &self.cache, dir);
        if !ahead.more || ahead.painted >= self.overscan {
            return;
        }
        let Some(num) = Self::block_at(items, self.view.anchor()) else {
            return;
        };
        self.ask.push(Ask::Fetch {
            num,
            older: dir == Dir::Older,
        });
    }

    /// A jump: the receipt is owed from here, and an empty target asks for its round. The
    /// newest round is already in hand; jumping to it would do nothing.
    fn ask_jump(&mut self, view: &View<'_>, items: &[Item<'_>]) {
        self.view.arm();
        let anchor = self.view.anchor();
        let Some(item) = items.iter().find(|it| it.num == anchor) else {
            return;
        };
        let Some(id) = item.block.id() else {
            return;
        };
        if self.cache.has_rows(anchor) {
            return;
        }
        let Some(turn) = view.turn(id) else {
            return;
        };
        if Self::newest_turn(view, items) == Some(turn) {
            return;
        }
        self.ask.push(Ask::Jump { turn });
    }

    /// The newest round the window holds: the round of its last block with an id.
    fn newest_turn(view: &View<'_>, items: &[Item<'_>]) -> Option<i64> {
        items
            .iter()
            .rev()
            .find_map(|it| it.block.id().and_then(|id| view.turn(id)))
    }

    /// One round older or newer: the sight line lands on that round's first block.
    fn round_step(&mut self, view: &View<'_>, items: &[Item<'_>], older: bool) {
        let anchor = self.view.anchor();
        let Some(here) = self.round_at(items, view, anchor) else {
            return;
        };
        let step = if older { -1 } else { 1 };
        let mut num = anchor;
        loop {
            if self.local.at(num + step).is_none() {
                return;
            }
            num += step;
            let Some(turn) = self.round_at(items, view, num) else {
                continue;
            };
            if turn == here {
                continue;
            }
            if older {
                while let Some(next) = self.round_at(items, view, num + step) {
                    if next != turn {
                        break;
                    }
                    num += step;
                }
            }
            self.view.focus_num(num);
            return;
        }
    }

    /// Put the sight line's round's first or last block under the sight line.
    fn align(&mut self, view: &View<'_>, items: &[Item<'_>], last: bool) {
        let anchor = self.view.anchor();
        let Some(id) = Self::block_at(items, anchor) else {
            return;
        };
        let Some(turn) = view.turn(id) else { return };
        if last && Self::newest_turn(view, items) == Some(turn) {
            return;
        }
        let rounds: Vec<i64> = items
            .iter()
            .filter(|it| it.block.id().and_then(|b| view.turn(b)) == Some(turn))
            .map(|it| it.num)
            .collect();
        let num = if last {
            rounds.iter().max()
        } else {
            rounds.iter().min()
        };
        if let Some(num) = num {
            self.view.focus_num(*num);
        }
    }
}

impl SubZone for HistoryZone {
    fn claims(&self) -> &'static [Claim] {
        CLAIMS
    }

    fn deliver_event(&mut self, ev: Event, ctx: &Ctx<'_>) {
        match ev {
            Event::Tick | Event::Deadline => {
                if self.view.note_still(self.silence) {
                    self.ask.push(Ask::Settled);
                }
            }
            Event::Resized => {
                let items = self.items(ctx.view);
                self.view.on_resize(&self.local, &items, &self.cache);
            }
            _ => {}
        }
    }

    fn deliver_action(&mut self, a: &Action, ctx: &Ctx<'_>) -> bool {
        let items = self.items(ctx.view);
        match a {
            Action::ScrollUp { shift } => {
                if *shift {
                    self.round_step(ctx.view, &items, true);
                    self.ask_jump(ctx.view, &items);
                } else {
                    self.view.step(&self.local, Dir::Older);
                }
                self.ask_more(&items, Dir::Older);
                true
            }
            Action::ScrollDown { shift } => {
                if *shift {
                    self.round_step(ctx.view, &items, false);
                    self.ask_jump(ctx.view, &items);
                } else {
                    self.view.step(&self.local, Dir::Newer);
                }
                self.ask_more(&items, Dir::Newer);
                true
            }
            Action::Key(Key { code, mods }) => match code {
                KeyCode::Char('t') if mods.ctrl => {
                    let thinking = !self.cache.thinking();
                    self.cache.set_switches(thinking, self.cache.tool());
                    self.view.on_resize(&self.local, &items, &self.cache);
                    true
                }
                KeyCode::Char('o') if mods.ctrl => {
                    let tool = !self.cache.tool();
                    self.cache.set_switches(self.cache.thinking(), tool);
                    self.view.on_resize(&self.local, &items, &self.cache);
                    true
                }
                KeyCode::Up if mods.ctrl => {
                    self.view.step(&self.local, Dir::Older);
                    self.ask_more(&items, Dir::Older);
                    true
                }
                KeyCode::Down if mods.ctrl => {
                    self.view.step(&self.local, Dir::Newer);
                    self.ask_more(&items, Dir::Newer);
                    true
                }
                KeyCode::PageUp if mods.ctrl => {
                    self.view.first(&self.local);
                    self.ask_jump(ctx.view, &items);
                    self.ask_more(&items, Dir::Older);
                    true
                }
                KeyCode::PageDown if mods.ctrl => {
                    self.view.follow();
                    self.view.arm();
                    true
                }
                KeyCode::PageUp => {
                    self.view.page(&self.local, &items, &self.cache, Dir::Older);
                    self.ask_more(&items, Dir::Older);
                    true
                }
                KeyCode::PageDown => {
                    self.view.page(&self.local, &items, &self.cache, Dir::Newer);
                    self.ask_more(&items, Dir::Newer);
                    true
                }
                KeyCode::Home if mods.ctrl && mods.shift => {
                    self.align(ctx.view, &items, false);
                    self.ask_jump(ctx.view, &items);
                    true
                }
                KeyCode::End if mods.ctrl && mods.shift => {
                    self.align(ctx.view, &items, true);
                    self.ask_jump(ctx.view, &items);
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn attach(&mut self, session: i64, _ctx: &Ctx<'_>) {
        self.open(session);
    }

    fn detach(&mut self, _ctx: &Ctx<'_>) {
        self.destroy();
    }

    fn request_height(&self, _term: TermSize) -> u16 {
        u16::MAX
    }

    fn assign(&mut self, _term: TermSize, rows: u16) {
        self.view.assign(rows);
    }

    fn on_height_changed(&mut self, rows: u16) -> bool {
        self.view.assign(rows);
        true
    }

    fn take_requests(&mut self) -> Vec<Request> {
        self.take_out().into_iter().map(Request::View).collect()
    }
}
