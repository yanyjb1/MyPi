// The history zone: what it holds, and the pane contract it answers.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::focus::Target;
use super::live::{Ask, Pin};
use super::local::section::{Idx, Local};
use super::local::slot::{Level, Life};
use super::prefetch::{Range, Run, Start};
use super::product::{Opts, Task, TodoPanel};
use super::{display, keys, wire};
use crate::ui::tui::lang::Lang;

use super::super::pane::Pane;
use crate::ui::tui::TodoFold;
use crate::ui::tui::action::Action;
use crate::ui::tui::theme::Accent;
use crate::ui::tui::zone::{Claim, Frame, Kind, Shared, Shout, Structure, TermSize, name};

// The classes the zone reads.
const KINDS: &[Kind] = &[Kind::Call, Kind::Stream, Kind::Notice];

// The sight line until the config answers.
const DEFAULT_FOCUS_LINE: u16 = 3;

// Blocks kept in the product layer until the config answers.
const DEFAULT_MARGIN: usize = 64;

// Rows a card head or tail may take until the config answers.
const DEFAULT_FOLD: usize = 10;

// Rows one wheel notch walks until the config answers.
const DEFAULT_SCROLL: usize = 10;

// Blocks kept in hand past the screen's edge until the config answers.
const DEFAULT_OVERSCAN: usize = 12;

// The config keys this zone asks for once per attachment.
pub(super) const CONFIG_KEYS: &[&str] = &[
    Lang::KEY,
    "config.ui.tui.zone.main.history.focusLine",
    "config.ui.tui.zone.main.history.cardFoldBudget",
    "config.ui.tui.zone.main.history.cacheMargin",
    "config.ui.tui.zone.main.history.scrollHeight",
    "config.ui.tui.zone.main.history.overscan",
    "config.ui.tui.zone.main.history.todoFold",
    "config.server.agent.tools.shell",
];

// The chat history: what the reader sees of the conversation.
pub struct HistoryZone {
    // The local section: render order and the content behind it.
    pub(super) local: Local,
    // The laid segments around the screen.
    pub(super) run: Run,
    // Where the screen starts.
    pub(super) range: Range,
    // How many blocks may keep their content and product.
    pub(super) margin: usize,
    // Positions whose content changed since the last draw.
    pub(super) dirty: BTreeSet<Idx>,
    // Positions whose text grew in place: the piece the run lays again, and everything after it.
    pub(super) grown: BTreeMap<Idx, usize>,
    // Blocks the last draw lacked; the next tick asks for their rounds.
    pub(super) missing: Vec<i64>,
    // The checklist's own product.
    pub(super) todo_panel: TodoPanel,
    pub(super) todo: Vec<Task>,
    pub(super) todo_fold: TodoFold,
    // What is still running, in the order it started; only the first one shows.
    pub(super) pin: Vec<Pin>,
    // The phase the spinner reads.
    pub(super) tick: usize,
    // The last width the layout was built at.
    pub(super) width: usize,
    // The rows the host granted.
    pub(super) height: u16,
    pub(super) focus_line: usize,
    pub(super) thinking: bool,
    pub(super) tool: bool,
    pub(super) fold: usize,
    pub(super) scroll_height: usize,
    // Blocks kept in hand past the screen's edge in the direction the reader moves.
    pub(super) overscan: usize,
    pub(super) placeholder: String,
    pub(super) cwd: String,
    // The highlight language the shell backend speaks; `None` until the config answers.
    pub(super) shell: Option<&'static str>,
    // The structural window as it last stood.
    pub(super) structure: Structure,
    // The language the banners and the numbers read in.
    pub(super) lang: Lang,
    // The compaction the server took: its clock, while it runs.
    pub(super) compacting: Option<Instant>,
    // The context as it stood before this compaction.
    pub(super) before: Option<u64>,
    // The context the newest compaction left behind, as the gauge estimates it.
    pub(super) after: Option<u64>,
    // What the outstanding context ask is for.
    pub(super) ask: Option<Ask>,
    // The blocks of a round whose aftermath has not arrived yet.
    pub(super) awaiting: Vec<i64>,
    // The id ranges asked for and not yet answered.
    pub(super) asking: HashSet<[i64; 2]>,
    // Echoes that arrived while the running region was owed a refetch: they belong to the next round, and drawing them now would let the refetch's wipe take them.
    pub(super) held_echo: Vec<String>,
    // A closed turn whose receipt does not line up: the ranges its own rows must come from.
    pub(super) pending: Option<Vec<[i64; 2]>>,
    // A fresh attach lays the structure out again.
    pub(super) needs_layout: bool,
    // A round is running: the statusline's busy glyph is on.
    pub(super) busy: bool,
    // The place the window is waiting to point at, once its content is here.
    pub(super) pending_focus: Option<Target>,
    // The post box, the palette and the draft every zone shares.
    pub(super) shared: Shared,

    // The conversation this zone follows; `None` = a draft.
    pub(super) session: Option<i64>,
    // What this frame owes the other zones.
    pub(super) shouts: Vec<Shout>,
    // Bumped whenever this zone's rows would differ.
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
            overscan: DEFAULT_OVERSCAN,
            placeholder: "thinking…".to_string(),
            cwd: String::new(),
            shell: None,
            structure: Structure::default(),
            lang: Lang::default(),
            compacting: None,
            before: None,
            after: None,
            ask: None,
            awaiting: Vec::new(),
            asking: HashSet::new(),
            held_echo: Vec::new(),
            pending: None,
            needs_layout: false,
            busy: false,
            pending_focus: None,
            shared: Shared::default(),
            session: None,
            shouts: Vec::new(),
            rev: 0,
        }
    }
}

impl HistoryZone {
    // Run one operation that needs the local section, the run and the range at once.
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
            lang,
            shell,
            ..
        } = self;
        let opts = Opts {
            thinking: *thinking,
            tool: *tool,
            placeholder: placeholder.as_str(),
            cwd: cwd.as_str(),
            fold: *fold,
            lang: *lang,
            shell: *shell,
        };
        f(local, run, range, &opts)
    }

    // The screen: the transcript, the checklist and the running label, in one run.
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
        self.with_draw(|local, run, _, _| {
            run.trim(top, top + height);
            let reach = run.reach();
            local.set_reach(reach);
        });
        self.missing = spend.missing;
        display::lines(&spend.rows, accent)
    }

    // One round's side effects, before the draw: the dirty set settles, and what the last draw lacked leaves as a request.
    pub(in crate::ui::tui::zone::main) fn tick(&mut self) {
        self.settle_dirty();
        let missing = std::mem::take(&mut self.missing);
        for block in missing {
            if let Some(turn) = self.turn_of(block) {
                self.ask_turn(turn);
            }
        }
    }

    // Positions whose content changed lose their rows once; content past the budget leaves.
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
        // The reader's window: the places the run laid, the place the range starts, and one overscan on either side; without it a fetch is dropped before the draw can lay it.
        let window = {
            let mut span = self.run.reach();
            if let Start::At(id, _) = self.range.start()
                && let Idx::Fixed(num) = id.idx
            {
                span = Some(match span {
                    Some((lo, hi)) => (lo.min(num), hi.max(num)),
                    None => (num, num),
                });
            }
            let pad = self.overscan as i64;
            span.map(|(lo, hi)| (lo - pad, hi + pad))
        };
        let HistoryZone {
            local,
            run,
            margin,
            asking,
            pending,
            missing,
            ..
        } = self;
        let evicted = local.evict(*margin, |id, num| {
            run.holds(Idx::Fixed(num))
                || missing.contains(&id)
                || window.is_some_and(|(lo, hi)| (lo..=hi).contains(&num))
                || asking.iter().any(|[lo, hi]| (*lo..=*hi).contains(&id))
                || pending
                    .as_ref()
                    .is_some_and(|ranges| ranges.iter().any(|[lo, hi]| (*lo..=*hi).contains(&id)))
        });
        for num in evicted {
            run.forget(Idx::Fixed(num));
        }
    }

    // One position's text grew in place: the run lays its tail again, the product stays.
    pub(super) fn grow(&mut self, idx: Idx, from: usize) {
        self.grown
            .entry(idx)
            .and_modify(|at| *at = (*at).min(from))
            .or_insert(from);
    }

    // One position's content changed: its rows are laid again at the next draw.
    pub(super) fn mark(&mut self, idx: Idx) {
        self.dirty.insert(idx);
        if let Some(card) = self.local.card_at(idx) {
            self.dirty.insert(card);
        }
    }

    // One fixed position's content changed: its rows go.
    pub(super) fn forget(&mut self, num: i64) {
        self.mark(Idx::Fixed(num));
    }

    // A line this front end has to say, in the mutable region.
    pub fn notice(&mut self, text: String, level: Level, life: Life) {
        for num in self.local.kill_short() {
            self.forget(num);
        }
        let idx = self.local.notice(text, level, life);
        self.mark(idx);
    }

    // `/new`: drop this conversation's section; the next attach asks for the new one.
    pub fn reset(&mut self) {
        self.destroy();
    }

    // The local section goes, and the run, the checklist and the range with it.
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
        self.compacting = None;
        self.before = None;
        self.after = None;
        self.ask = None;
        self.awaiting.clear();
        self.held_echo.clear();
    }

    // One user line this front end or another wrote: it draws at once, unless the running region is still owed a refetch — that wipe would take it, so it waits for the next round.
    pub(super) fn echo(&mut self, text: &str) {
        if self.pending.is_some() {
            self.held_echo.push(text.to_string());
            return;
        }
        let at = self.local.running_mut().echo(text);
        self.mark(Idx::Turn(at as u32));
        self.shouts
            .push(Shout::new(name::TICK, serde_json::json!({})));
    }

    // The refetch wiped the running region: the lines held back take their places in the empty round now.
    pub(super) fn release_echo(&mut self) {
        for text in std::mem::take(&mut self.held_echo) {
            self.echo(&text);
        }
    }

    // The three shared handles arrive once, at boot.
    pub(in crate::ui::tui::zone::main) fn bind(&mut self, shared: Shared) {
        self.shared = shared;
    }

    // The session's own row.
    pub(in crate::ui::tui::zone::main) fn ask_meta(&self) {
        if let Some(session) = self.session {
            self.shared.mail.send(Frame::new(
                "call",
                "meta",
                serde_json::json!({"session": session}),
            ));
        }
    }

    // One configuration key.
    pub(in crate::ui::tui::zone::main) fn want(&self, key: &str) {
        self.shared.mail.send(Frame::new(
            "call",
            "config",
            serde_json::json!({"key": key}),
        ));
    }

    // The session's bill, after a round moved it.
    pub(in crate::ui::tui::zone::main) fn bill(&self) {
        if let Some(session) = self.session {
            self.shared.mail.send(Frame::new(
                "call",
                "usage",
                serde_json::json!({"session": session}),
            ));
        }
    }

    // The conversation as it stands: every round, the newest blocks and the checklist.
    pub(in crate::ui::tui::zone::main) fn ask_init(&self) {
        if let Some(session) = self.session {
            self.shared.mail.send(Frame::new(
                "call",
                "init",
                serde_json::json!({"session": session}),
            ));
        }
    }

    // One receipt's rows.
    pub(in crate::ui::tui::zone::main) fn want_blocks(&self, ranges: &[[i64; 2]]) {
        if let Some(session) = self.session {
            self.shared.mail.send(Frame::new(
                "call",
                "blocks",
                serde_json::json!({"session": session, "ranges": ranges}),
            ));
        }
    }
}

impl HistoryZone {
    fn bump(&mut self) {
        self.rev = self.rev.wrapping_add(1);
    }

    // How often the ESC row changes on its own: the starburst turns 30 times a second.
    pub(crate) fn anim(&self) -> Option<Duration> {
        (!self.pin.is_empty()).then(|| Duration::from_millis(1000 / 30))
    }
}

impl HistoryZone {
    pub(in crate::ui::tui::zone::main) fn claims(&self) -> &'static [Claim] {
        keys::CLAIMS
    }

    pub(in crate::ui::tui::zone::main) fn kinds(&self) -> &'static [Kind] {
        KINDS
    }

    pub(in crate::ui::tui::zone::main) fn revision(&self) -> u64 {
        self.rev
    }

    pub(in crate::ui::tui::zone::main) fn deliver_event(&mut self, frame: &Frame, _size: TermSize) {
        wire::read_frame(self, frame);
        self.bump();
    }

    pub(in crate::ui::tui::zone::main) fn deliver_action(&mut self, action: &Action) -> bool {
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
            Action::Key(key) => keys::press(self, *key),
        };
        if handled {
            self.ensure_around();
        }
        self.bump();
        handled
    }

    pub(in crate::ui::tui::zone::main) fn attach(&mut self, session: i64, _size: TermSize) {
        self.session = Some(session);
        self.destroy();
        self.needs_layout = true;
        self.range.follow();
        for key in CONFIG_KEYS {
            self.want(key);
        }
        // A restored session shows the newest compaction's aftermath, if that is still where the gauge stands.
        self.ask = Some(Ask::Attach);
        self.ask_init();
        self.ask_meta();
        self.bill();
        self.bump();
    }

    pub(in crate::ui::tui::zone::main) fn detach(&mut self) {
        self.session = None;
        self.destroy();
        self.bump();
    }

    pub(in crate::ui::tui::zone::main) fn take_shouts(&mut self) -> Vec<Shout> {
        std::mem::take(&mut self.shouts)
    }

    pub(in crate::ui::tui::zone::main) fn hear(&mut self, event: &str, content: &Value) {
        match event {
            name::COMPACT_START => self.compact_started(),
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
                    self.echo(text);
                }
            }
            name::RETRACT => {
                if self.held_echo.pop().is_none()
                    && let Some(at) = self.local.running_mut().retract()
                {
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
