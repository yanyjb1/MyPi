//! The desk: one temp hand at a time, its frozen report, its keys, its rows.
//!
//! The input zone calls a hand in; the desk keeps no state of its own beyond the slot.

use super::SubZone;
use crate::ui::tui::action::Action;
use crate::ui::tui::zone::{Claim, Ctx, Event, Request, TermSize};

pub mod hand;
pub mod hands;

pub use hand::Reply;
use hand::{Hand, IDLE_ROWS, Report};

/// Who the input zone calls in. Also the roster key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hire {
    Completion,
    Usage,
}

/// The desk.
#[derive(Default)]
pub struct ReservedZone {
    completion: hands::completion::CompletionService,
    usage: hands::usage::UsagePanel,
    /// Who is on duty now.
    on_duty: Option<Hire>,
    /// The report taken the first frame after arrival: height and keys frozen here.
    report: Option<Report>,
    /// Rows the zone allocated this frame.
    want: u16,
    /// A reply for the editor, waiting for the zone to take it.
    reply: Option<Reply>,
    /// The last numbers handed to the usage panel.
    last_usage: Option<(
        crate::server::api::types::UsageLang,
        Vec<crate::server::api::call::usage::ModelUsage>,
    )>,
}

impl ReservedZone {
    /// Nothing is claimed up front; the desk only borrows.
    pub const CLAIMS: &[Claim] = &[];

    /// The hand for one hire signal. One `match`: a new hand touches here only.
    fn hand(&mut self, who: Hire) -> &mut dyn Hand {
        match who {
            Hire::Completion => &mut self.completion,
            Hire::Usage => &mut self.usage,
        }
    }

    /// Call a hand in. Whoever is on duty leaves first.
    pub fn hire(&mut self, who: Hire, ctx: &Ctx) {
        if let Some(prev) = self.on_duty
            && prev != who
        {
            self.hand(prev).left();
        }
        self.on_duty = Some(who);
        self.report = None;
        self.hand(who).arrived(ctx);
    }

    /// The editor's text and cursor, after every edit.
    pub fn text(&mut self, text: &str, cursor: usize) {
        let wants = self.report.is_some_and(|r| r.text);
        if wants && let Some(who) = self.on_duty {
            self.hand(who).text(text, cursor);
        }
    }

    /// The cursor's column, after every draw.
    pub fn col(&mut self, col: u16) {
        let wants = self.report.is_some_and(|r| r.col);
        if wants && let Some(who) = self.on_duty {
            self.hand(who).col(col);
        }
    }

    /// A reply the editor has to apply.
    pub fn take_reply(&mut self) -> Option<Reply> {
        self.reply.take()
    }

    /// The panel's display options, read from the config once at assembly.
    pub fn set_usage_config(&mut self, cfg: &crate::server::config::read::config::UsageConfig) {
        self.usage.set_align(cfg.align);
    }

    /// The panel's numbers arrived.
    pub fn show_usage(
        &mut self,
        lang: crate::server::api::types::UsageLang,
        groups: Vec<crate::server::api::call::usage::ModelUsage>,
    ) {
        self.usage.show(lang, groups);
    }

    /// Feed the panel only when the numbers actually moved.
    pub fn sync_usage(&mut self, view: &crate::ui::session::View<'_>) {
        let Some(usage) = view.usage else { return };
        let same = self.last_usage.as_ref().is_some_and(|(lang, groups)| {
            *lang == usage.lang && groups.as_slice() == usage.groups.as_slice()
        });
        if same {
            return;
        }
        self.last_usage = Some((usage.lang, usage.groups.clone()));
        self.show_usage(usage.lang, usage.groups.clone());
    }

    /// Frame start: drop a hand that left, take the report of a fresh one.
    pub(super) fn settle(&mut self) {
        let Some(who) = self.on_duty else {
            return;
        };
        if !self.hand(who).stays() {
            self.hand(who).left();
            self.on_duty = None;
            self.report = None;
            return;
        }
        if self.report.is_none() {
            let ready = self.hand(who).ready();
            if ready {
                let report = self.hand(who).report();
                self.report = Some(report);
            }
        }
    }
}

impl SubZone for ReservedZone {
    fn claims(&self) -> &'static [Claim] {
        Self::CLAIMS
    }

    fn borrowed(&self) -> &'static [Claim] {
        self.report.map(|r| r.keys).unwrap_or(&[])
    }

    fn deliver_event(&mut self, ev: Event, ctx: &Ctx) {
        self.settle();
        if let Some(who) = self.on_duty {
            self.hand(who).on_event(ev, ctx);
        }
    }

    fn deliver_action(&mut self, a: &Action, _ctx: &Ctx) -> bool {
        self.settle();
        let Action::Key(key) = a else {
            return false;
        };
        let Some(who) = self.on_duty else {
            return false;
        };
        if let Some(reply) = self.hand(who).on_key(key) {
            self.reply = Some(reply);
        }
        true
    }

    fn attach(&mut self, _session: i64, _ctx: &Ctx) {}

    fn detach(&mut self, _ctx: &Ctx) {
        if let Some(who) = self.on_duty {
            self.hand(who).left();
        }
        self.on_duty = None;
        self.report = None;
        self.reply = None;
        self.want = 0;
        self.last_usage = None;
    }

    fn take_requests(&mut self) -> Vec<Request> {
        Vec::new()
    }

    fn request_height(&self, _term: TermSize) -> u16 {
        self.report.map(|r| r.rows).unwrap_or(IDLE_ROWS)
    }

    fn assign(&mut self, _term: TermSize, rows: u16) {
        self.want = rows.max(IDLE_ROWS);
    }

    fn on_height_changed(&mut self, _rows: u16) -> bool {
        false
    }
}

impl ReservedZone {
    /// Idle: one blank row. On duty: the hand draws its own rows.
    pub fn render_rows(
        &mut self,
        term_w: u16,
        view: &crate::ui::session::View<'_>,
    ) -> Vec<ratatui::text::Line<'static>> {
        self.settle();
        let rows = usize::from(self.want.max(IDLE_ROWS));
        let Some(who) = self.on_duty else {
            return vec![ratatui::text::Line::from("")];
        };
        let ctx = Ctx {
            view,
            size: TermSize {
                cols: term_w,
                rows: self.want,
            },
            memo: None,
        };
        self.hand(who).draw(&ctx, rows)
    }
}
