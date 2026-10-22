//! The session picker: a full-screen list of stored sessions.
//!
//! Rows are read from the session view; what the page wants done is queued for the loop.

use crate::server::api::call::sessions::SessionRow;
use crate::ui::session::View;
use crate::ui::tui::action::{Action, KeyCode};
use crate::ui::tui::zone::{Ctx, Event, Handover, Request, TermSize, Zone, ZoneId};

mod render;

/// Which sessions the list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Only sessions that ran under the current directory.
    Current,
    /// Every session on this machine.
    All,
}

impl Scope {
    fn toggled(self) -> Self {
        match self {
            Scope::Current => Scope::All,
            Scope::All => Scope::Current,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Scope::Current => "当前目录",
            Scope::All => "全部目录",
        }
    }
}

pub struct ResumeZone {
    size: Option<TermSize>,
    /// Indices that passed the filter: filtering keeps indices, not rows.
    visible: Vec<usize>,
    /// Row count at the last sync: the visible set is recomputed only when it moves.
    seen: usize,
    filter: String,
    /// Position inside `visible`.
    selected: usize,
    /// First visible row (written back by the render).
    scroll: usize,
    scope: Scope,
    cwd: std::path::PathBuf,
    /// Whether a list ever arrived: tells "empty" from "not yet".
    loaded: bool,
    /// The row a Delete armed; Enter confirms, anything else cancels.
    pending_delete: Option<i64>,
    /// What the loop still has to do. A queue, not a slot: a delete refetches the list too.
    pending: Vec<Request>,
}

impl Default for ResumeZone {
    fn default() -> Self {
        Self {
            size: None,
            visible: Vec::new(),
            seen: usize::MAX,
            filter: String::new(),
            selected: 0,
            scroll: 0,
            scope: Scope::Current,
            cwd: std::path::PathBuf::from("."),
            loaded: false,
            pending_delete: None,
            pending: Vec::new(),
        }
    }
}

impl ResumeZone {
    /// Enter the page: remember the directory and ask for the list.
    ///
    /// The list is refetched every time: it is a fact about the outside, and last time's
    /// snapshot may be stale.
    pub fn begin(&mut self, cwd: std::path::PathBuf) {
        self.cwd = cwd;
        self.loaded = false;
        self.scroll = 0;
        self.pending_delete = None;
        self.filter.clear();
        self.pending.push(Request::List {
            under: self.under(),
        });
    }

    /// Follow the view: recompute the visible set only when the row count moved.
    pub fn sync(&mut self, view: &View<'_>) {
        let rows = view.rows.unwrap_or(&[]);
        self.loaded = view.rows.is_some();
        if rows.len() != self.seen {
            self.seen = rows.len();
            self.refilter(rows);
        }
    }

    fn under(&self) -> Option<std::path::PathBuf> {
        match self.scope {
            Scope::Current => Some(self.cwd.clone()),
            Scope::All => None,
        }
    }

    fn row<'a>(&self, view: &'a View<'_>) -> Option<&'a SessionRow> {
        let i = *self.visible.get(self.selected)?;
        view.rows?.get(i)
    }

    /// The selected session id, taken through the filter: the index into the original list
    /// is not the index on screen.
    pub fn selected_id(&self, view: &View<'_>) -> Option<i64> {
        self.row(view).map(|r| r.meta.id)
    }

    /// Recompute the visible set. The filter matches title, name, first message and
    /// directory, case-insensitively.
    fn refilter(&mut self, rows: &[SessionRow]) {
        let needle = self.filter.to_lowercase();
        self.visible = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| needle.is_empty() || haystack(r).contains(&needle))
            .map(|(i, _)| i)
            .collect();
        self.selected = self.selected.min(self.visible.len().saturating_sub(1));
        self.scroll = 0;
    }

    fn move_by(&mut self, delta: i64) {
        if self.visible.is_empty() {
            return;
        }
        let last = self.visible.len() - 1;
        let next = self.selected as i64 + delta;
        self.selected = next.clamp(0, last as i64) as usize;
    }
}

/// Everything about one session the filter can match on.
pub(crate) fn haystack(row: &SessionRow) -> String {
    format!(
        "{} {} {} {}",
        row.meta.id,
        row.meta.name.as_deref().unwrap_or(""),
        row.first_user.as_deref().unwrap_or(""),
        row.meta.cwd.as_deref().unwrap_or("")
    )
    .to_lowercase()
}

/// The first line: an explicit name, else the first user message, else a synthesized label.
pub(crate) fn title(row: &SessionRow) -> String {
    if let Some(name) = &row.meta.name {
        return name.clone();
    }
    if let Some(first) = preview_text(row) {
        return first;
    }
    crate::server::api::call::sessions::display_name(&row.meta, row.first_user.as_deref())
}

/// The preview line: only named sessions draw their first message (it is the title otherwise).
pub(crate) fn preview(row: &SessionRow) -> Option<String> {
    row.meta.name.as_ref()?;
    preview_text(row)
}

/// The first line of the first user message; multi-line previews are not drawn.
pub(crate) fn preview_text(row: &SessionRow) -> Option<String> {
    let text = row.first_user.as_deref()?;
    let first = text.lines().next().unwrap_or("").trim();
    (!first.is_empty()).then(|| first.to_string())
}

impl Zone for ResumeZone {
    fn deliver_event(&mut self, ev: Event, ctx: &Ctx) {
        self.size = Some(ctx.size);
        if ev == Event::Resized {
            self.scroll = 0;
        }
    }

    fn deliver_action(&mut self, a: &Action, ctx: &Ctx) -> Option<Handover> {
        self.size = Some(ctx.size);
        let rows = ctx.view.rows.unwrap_or(&[]);
        match a {
            Action::ScrollUp { .. } => self.move_by(-1),
            Action::ScrollDown { .. } => self.move_by(1),
            Action::Paste(text) => {
                // A paste is a filter word here: a picker is not worth multi-line semantics.
                self.filter.push_str(&text.replace('\n', " "));
                self.refilter(rows);
            }
            Action::Key(key) => {
                let ctrl = key.mods.ctrl;
                // Any other operation cancels an armed delete; only Enter keeps it.
                if !matches!(key.code, KeyCode::Enter | KeyCode::Delete | KeyCode::Esc) {
                    self.pending_delete = None;
                }
                match key.code {
                    KeyCode::Delete => {
                        self.pending_delete = self.selected_id(ctx.view);
                        return None;
                    }
                    KeyCode::Esc => {
                        // Hand the screen back: the page leaves no trace either way.
                        return Some(Handover {
                            to: Some(ZoneId::Main),
                            requests: self.take_requests(),
                        });
                    }
                    KeyCode::Enter => {
                        let id = self.selected_id(ctx.view)?;
                        if self.pending_delete == Some(id) {
                            // The prompt stays up: a refused delete turns it yellow, a landed
                            // one takes its row with it.
                            self.pending.push(Request::Delete(id));
                            self.pending.push(Request::List {
                                under: self.under(),
                            });
                            return None;
                        }
                        self.pending.push(Request::Attach(id));
                        return Some(Handover {
                            to: Some(ZoneId::Main),
                            requests: self.take_requests(),
                        });
                    }
                    KeyCode::Up => self.move_by(-1),
                    KeyCode::Down => self.move_by(1),
                    KeyCode::PageUp => self.move_by(-PAGE),
                    KeyCode::PageDown => self.move_by(PAGE),
                    KeyCode::Home => self.selected = 0,
                    KeyCode::End => self.selected = self.visible.len().saturating_sub(1),
                    KeyCode::Tab => {
                        self.scope = self.scope.toggled();
                        self.pending.push(Request::List {
                            under: self.under(),
                        });
                    }
                    KeyCode::Backspace => {
                        self.filter.pop();
                        self.refilter(rows);
                    }
                    KeyCode::Char('u') if ctrl => {
                        self.filter.clear();
                        self.refilter(rows);
                    }
                    KeyCode::Char(c) if !ctrl => {
                        self.filter.push(c);
                        self.refilter(rows);
                    }
                    _ => {}
                }
            }
        }
        None
    }

    fn take_over(&mut self, ctx: &Ctx) {
        self.size = Some(ctx.size);
        self.begin(ctx.view.cwd.into());
    }

    fn detach(&mut self, _ctx: &Ctx) {
        self.size = None;
        self.visible.clear();
        self.seen = usize::MAX;
        self.filter.clear();
        self.selected = 0;
        self.scroll = 0;
        self.loaded = false;
        self.pending_delete = None;
        self.pending.clear();
    }

    fn render(&mut self, ctx: &Ctx) -> Vec<ratatui::text::Line<'static>> {
        self.sync(ctx.view);
        render::lines(self, ctx.view)
    }

    fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.pending)
    }
}

impl ResumeZone {
    pub(crate) fn scope(&self) -> Scope {
        self.scope
    }

    pub(crate) fn scope_label(&self) -> &'static str {
        self.scope.label()
    }

    pub(crate) fn filter(&self) -> &str {
        &self.filter
    }

    /// The row a Delete armed (the render draws the prompt after the title).
    pub(crate) fn pending_delete(&self) -> Option<i64> {
        self.pending_delete
    }

    pub(crate) fn loaded(&self) -> bool {
        self.loaded
    }

    pub(crate) fn visible_len(&self) -> usize {
        self.visible.len()
    }

    pub(crate) fn selected(&self) -> usize {
        self.selected
    }

    pub(crate) fn set_scroll(&mut self, top: usize) {
        self.scroll = top;
    }

    pub(crate) fn scroll(&self) -> usize {
        self.scroll
    }

    pub(crate) fn size(&self) -> Option<TermSize> {
        self.size
    }

    /// Row `i` of the visible list, as it stands in the view.
    pub(crate) fn row_at<'a>(&self, i: usize, view: &'a View<'_>) -> Option<&'a SessionRow> {
        let k = *self.visible.get(i)?;
        view.rows?.get(k)
    }
}

/// Rows per page.
const PAGE: i64 = 8;
