//! The input zone: what it holds, and the two contracts it answers.

use std::collections::VecDeque;
use std::time::Instant;

use serde_json::{Value, json};

use super::super::desk::Hire;
use super::super::geometry;
use super::super::pane::Pane;
use super::boot::DEFAULT_MODEL_KEY;
use super::ctx::{Ctx, Data};
use super::draft::Draft;
use super::editor::{Editor, History};
use super::exit::ExitRequest;
use super::notice::Notice;
use super::queue::Queue;
use super::render;
use super::semantics;
use super::statusline::{StatusLine, StatusTheme};
use super::submit::Waiting;
use crate::ui::tui::action::{Action, KeyCode};
use crate::ui::tui::theme::Accent;
use crate::ui::tui::zone::Frame;
use crate::ui::tui::zone::{Claim, Kind, Shared, Shout, SubZone, TermSize, name};

/// The input sub-zone. Its height comes from its own content.
pub struct InputZone {
    /// The post box and the palette every zone shares.
    pub(super) shared: Shared,

    /// The draft a session opens with; only this zone keeps one.
    pub(super) draft: Draft,

    /// Input viewport start (wrapped row). Independent of the cursor.
    pub scroll: usize,

    /// The height it declares: editor rows + statusline + border.
    pub rows: u16,

    /// The editor itself: text, cursor, undo.
    pub editor: Editor,

    /// Input history: an in-memory ring, 20 entries, never stored.
    pub history: History,

    /// Idle and rewinding: the box is empty, so ↑↓ walks the history.
    pub finished: bool,

    /// The cursor sits in a trigger word: the condition for lending keys.
    pub hang: bool,

    /// The hand to hire, produced here and taken by the zone.
    pub(super) pending_hire: Option<Hire>,

    /// The last ↑↓ goal column, carried for `Editor::up` / `down`.
    pub goal_col: Option<usize>,

    /// Exit requests, in order: taken by the main zone.
    pub(super) pending_exits: Vec<ExitRequest>,

    /// Sentences sent while a round was running, head first.
    pub(super) queue: Queue,

    /// The sentence a submit put out, kept only until its reply says it opened nothing.
    pub(super) in_flight: Option<String>,

    /// The message whose opening `post init` is out; it leaves as soon as the row answers.
    pub(super) pending_open: Option<(Option<String>, String)>,
    /// A preparation waits for this session's first message, or the first after a compaction.
    pub(super) prepare_pending: bool,

    /// The structure snapshot of this attachment has been read.
    pub(super) struct_seen: bool,

    /// The newest compaction round this attachment has seen.
    pub(super) last_compact: Option<i64>,

    /// The line that opens a draft is out: the structure that answers `init` is not a second primer.
    pub(super) opening_sent: bool,

    /// When the last un-lent Esc fired: another one inside the window opens the tree.
    pub(super) esc_armed: Option<Instant>,

    /// When a sentence last entered the mailbox: the next Enter interrupts.
    pub(super) enter_armed: Option<Instant>,

    /// Notices waiting for the receipt that says their write landed.
    pub(super) pending_notice: VecDeque<Waiting>,

    /// The cursor's absolute terminal column, border included.
    pub cursor_col: u16,

    /// The cursor's row inside the container (0 = the statusline row).
    pub cursor_row: u16,

    /// The statusline: the top edge of the container and its own deco table.
    pub statusline: StatusLine,

    /// The session this zone follows; `None` = a draft.
    pub(super) session: Option<i64>,

    /// The startup questions were asked; one answer per run is enough.
    pub(super) booted: bool,

    /// The terminal size from this round's arbitration.
    pub(super) term: TermSize,

    /// What the parts read.
    pub(super) data: Data,

    /// Events this zone puts out.
    pub(super) shouts: Vec<Shout>,

    /// Bumped whenever this zone's rows would differ.
    rev: u64,

    /// The editor text wrapped at the width it was last laid at.
    view: crate::ui::tui::text::Wrapped,

    /// The `(columns, edit generation)` `view` was wrapped from.
    view_key: (usize, u64),

    /// Bumped on every edit: the wrap follows it.
    pub(super) wrap_gen: u64,
}

impl Default for InputZone {
    fn default() -> Self {
        Self {
            shared: Shared::default(),
            draft: Draft::default(),
            scroll: 0,
            rows: 0,
            editor: Editor::new(),
            history: History::new(),
            finished: true,
            hang: false,
            pending_hire: None,
            goal_col: None,
            pending_exits: Vec::new(),
            queue: Queue::default(),
            in_flight: None,
            pending_open: None,
            prepare_pending: true,
            struct_seen: false,
            last_compact: None,
            opening_sent: false,
            esc_armed: None,
            enter_armed: None,
            pending_notice: VecDeque::new(),
            cursor_col: geometry::BORDER_COLS as u16,
            cursor_row: 1,
            statusline: StatusLine::new(),
            session: None,
            booted: false,
            term: TermSize { cols: 80, rows: 24 },
            data: Data::default(),
            shouts: Vec::new(),
            rev: 0,
            view: crate::ui::tui::text::Wrapped::default(),
            view_key: (0, 0),
            wrap_gen: 0,
        }
    }
}

impl InputZone {
    /// Everything the editor acts on, named in `claim.rs`.
    pub const CLAIMS: &'static [Claim] = super::claim::CLAIMS;

    /// The hand the editor asked for, if it asked this frame.
    pub fn take_hire(&mut self) -> Option<Hire> {
        self.pending_hire.take()
    }

    /// The exits waiting to be turned into asks.
    pub fn take_exits(&mut self) -> Vec<ExitRequest> {
        std::mem::take(&mut self.pending_exits)
    }

    /// The shared handles arrive once, at boot.
    pub(in crate::ui::tui::zone::main) fn bind(&mut self, shared: Shared) {
        self.shared = shared;
    }

    pub fn draft_moved(&mut self, cwd: &str) {
        self.draft.moved(cwd.into());
    }

    pub fn draft_cleared(&mut self) {
        self.draft.clear();
    }

    pub fn draft_cwd(&self) -> Option<String> {
        Some(self.draft.cwd().to_string_lossy().into_owned())
    }

    pub fn draft_set_model(&mut self, model: &str) {
        self.draft.set_model(model.to_string());
    }

    /// One configuration key.
    pub(super) fn want(&self, key: &str) {
        self.shared
            .mail
            .send(Frame::new("call", "config", json!({"key": key})));
    }

    /// The session's bill: the server is the only party that knows it.
    pub(super) fn bill(&self) {
        if let Some(session) = self.session {
            self.shared
                .mail
                .send(Frame::new("call", "usage", json!({"session": session})));
        }
    }

    /// One frame for the statusline: a deco with work of its own runs it here.
    pub fn tick_statusline(&mut self) {
        let ctx = Ctx { data: &self.data };
        if self.statusline.tick(&ctx) {
            self.bump();
        }
    }

    pub(super) fn bump(&mut self) {
        self.rev = self.rev.wrapping_add(1);
    }

    /// Esc first (a lent-away Esc never arrives here), then the intent table.
    fn act(&mut self, a: &Action) -> bool {
        let key = match a {
            Action::Key(key) => *key,
            Action::Paste(text) => {
                self.enter_armed = None;
                let intent = semantics::Intent::Paste(text.clone());
                let consumed = self.apply(&intent, None);
                return self.edited(consumed);
            }
            Action::ScrollUp { .. } | Action::ScrollDown { .. } => return false,
        };
        if key.code == KeyCode::Esc && self.route_esc(&key) {
            return true;
        }
        let intent = semantics::translate(key);
        if matches!(intent, semantics::Intent::None) {
            return false;
        }
        // Only two Enters in a row interrupt: any other key spends the arm.
        if !matches!(intent, semantics::Intent::Submit) {
            self.enter_armed = None;
        }
        let wrapped = matches!(intent, semantics::Intent::Up | semantics::Intent::Down)
            .then(|| self.wrapped());
        let consumed = self.apply(&intent, wrapped.as_ref());
        self.edited(consumed)
    }

    /// The editor's text and where the cursor sits in it.
    pub fn draft(&self) -> (String, usize) {
        (self.editor.text(), self.editor.cursor())
    }

    /// The height it asks for: content rows, capped at a share of the terminal.
    fn request_height_for(&self, wrapped_lines: usize, term_h: u16) -> u16 {
        geometry::container_height(term_h, wrapped_lines)
            .try_into()
            .unwrap_or(u16::MAX)
    }

    /// The wrapped result at the current width: keys and rendering share it.
    fn ensure_view(&mut self, cols: u16) {
        let key = (usize::from(cols.max(1)), self.wrap_gen);
        if self.view_key != key {
            self.view =
                crate::ui::tui::text::wrap(&self.editor.text(), geometry::inner_width(cols));
            self.view_key = key;
        }
    }

    /// A copy of the wrapped text: the cursor-walking intents ask for one.
    fn wrapped(&mut self) -> crate::ui::tui::text::Wrapped {
        let cols = self.term.cols;
        self.ensure_view(cols);
        self.view.clone()
    }

    /// The statusline's own reading of an event.
    fn notify(&mut self, event: &str, content: &Value) {
        let ctx = Ctx { data: &self.data };
        self.statusline.on_event(event, content, &ctx);
    }

    /// One signal: a field this zone follows changed.
    fn signal(&mut self, content: &Value) {
        if let Some(address) = content.get("model").and_then(Value::as_str) {
            self.model(address);
        }
        if let Some(name) = content.get("name") {
            self.data.name = name.as_str().map(str::to_string);
        }
    }
}

impl SubZone for InputZone {
    fn claims(&self) -> &'static [Claim] {
        Self::CLAIMS
    }

    fn kinds(&self) -> &'static [Kind] {
        &[Kind::Call, Kind::Post, Kind::Subscribe]
    }

    fn revision(&self) -> u64 {
        self.rev
    }

    fn deliver_event(&mut self, frame: &Frame, size: TermSize) {
        self.term = size;
        self.read_frame(frame);
        self.bump();
    }

    /// Esc first (a lent-away Esc never arrives here), then the intent table.
    fn deliver_action(&mut self, a: &Action) -> bool {
        let consumed = self.act(a);
        self.bump();
        consumed
    }

    fn attach(&mut self, session: i64, _size: TermSize) {
        // A draft turning real carries its own live state; only a session
        // switch has old data to wipe.
        if self.session.is_some() {
            self.reset_data();
            self.opening_sent = false;
        }
        self.session = Some(session);
        self.prepare_pending = false;
        self.struct_seen = false;
        self.last_compact = None;
        self.queue.clear();
        self.in_flight = None;
        self.pending_notice.clear();
        // The line that opened this row was waiting for its id.
        if let Some((prefix, text)) = self.pending_open.take() {
            self.in_flight = Some(text.clone());
            // `history.attach` wiped the rows: the line that opened this one is drawn again here.
            self.shouts
                .push(Shout::new(name::ECHO, json!({"text": text.clone()})));
            self.shared.mail.send(Frame::new(
                "subscribe",
                "submit",
                json!({"session": session, "prefix": prefix, "text": text}),
            ));
            self.data.busy = true;
            self.shouts
                .push(Shout::new(name::BUSY, json!({"on": true})));
        }
        self.bill();
        self.bump();
    }

    fn detach(&mut self) {
        self.session = None;
        self.prepare_pending = false;
        self.struct_seen = false;
        self.last_compact = None;
        self.opening_sent = false;
        self.pending_exits.clear();
        self.pending_hire = None;
        self.hang = false;
        self.esc_armed = None;
        self.enter_armed = None;
        self.editor.clear();
        self.wrap_gen = self.wrap_gen.wrapping_add(1);
        self.history.exit();
        self.finished = true;
        self.scroll = 0;
        self.reset_data();
        self.queue.clear();
        self.in_flight = None;
        self.pending_notice.clear();
        self.want(DEFAULT_MODEL_KEY);
        self.bump();
    }

    fn take_shouts(&mut self) -> Vec<Shout> {
        std::mem::take(&mut self.shouts)
    }

    fn hear(&mut self, event: &str, content: &Value) {
        match event {
            name::CWD => {
                if let Some(path) = content.get("path").and_then(Value::as_str) {
                    self.data.cwd.clear();
                    self.data.cwd.push_str(path);
                    if let Some(temporary) = content.get("temporary").and_then(Value::as_bool) {
                        self.data.temporary = temporary;
                    }
                    if content.get("fallback").and_then(Value::as_bool) == Some(true) {
                        self.say(&Notice::CwdFallback);
                    }
                    self.notify(event, content);
                }
            }
            name::USAGE => {
                // A request landed: the answer is the server's to give, live figures and all.
                self.bill();
            }
            name::TICK => {
                self.notify(event, content);
            }
            name::PREFILL => {
                if let Some(text) = content.get("text").and_then(Value::as_str) {
                    self.prefill(text);
                }
            }
            name::BUSY => {
                self.data.busy = content.get("on").and_then(Value::as_bool).unwrap_or(false);
                if !self.data.busy {
                    self.enter_armed = None;
                    self.bill();
                }
                self.notify(event, content);
            }
            name::MAILBOX => {
                self.opened();
            }
            name::SIGNAL => {
                self.signal(content);
            }
            _ => {}
        }
        self.bump();
    }
}

impl Pane for InputZone {
    fn request_height(&mut self, term: TermSize) -> u16 {
        self.ensure_view(term.cols);
        self.request_height_for(self.view.len(), term.rows)
    }

    fn assign(&mut self, term: TermSize, rows: u16) {
        self.term = term;
        if self.rows != rows {
            self.rows = rows;
            self.bump();
        }
    }
}

impl InputZone {
    /// Self-rendering; the width comes in as an argument, like its siblings.
    ///
    /// Order: wrap, scroll to the cursor, the statusline row, then the container.
    pub fn render_rows(
        &mut self,
        term: TermSize,
        accent: &Accent,
    ) -> Vec<ratatui::text::Line<'static>> {
        self.term = term;
        let term_w = term.cols.max(1);
        self.ensure_view(term_w);
        let (cursor_row, cursor_col) = self.view.locate(self.editor.cursor());
        let container_rows = usize::from(self.rows.max(2));
        self.scroll =
            geometry::adjust_scroll(self.scroll, self.view.len(), container_rows, cursor_row);
        let status_line = {
            let ctx = Ctx { data: &self.data };
            self.statusline
                .render(&StatusTheme::new(*accent), term_w, &ctx)
        };
        let drawn = render::render(
            &render::Spec {
                status_line,
                wrapped: &self.view,
                starts: self.scroll,
                container_rows,
                cursor_row,
                cursor_col,
                term_w,
            },
            accent,
        );
        self.cursor_col = drawn.cursor_col as u16;
        self.cursor_row = drawn.cursor_row as u16;
        drawn.lines
    }
}
