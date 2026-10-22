//! The app: which zone shows, and the only path that changes it.

mod handover;
mod id;
mod lifecycle;
mod memo;
mod startup;

pub use id::Uuid;
pub use memo::{Memo, Operator};
pub use startup::Startup;

use crate::ui::tui::action::Action;
use crate::ui::tui::event::Event;
use crate::ui::tui::zone::{
    Ctx, Handover, MainZone, Ownership, Request, ResumeZone, TreeZone, Zone, ZoneId,
};

/// The ownership ledger and the zone registry.
pub struct App {
    id: Uuid,
    memo: Memo,
    session: Option<i64>,
    ownership: Ownership,
    main: MainZone,
    resume: ResumeZone,
    tree: TreeZone,
    carried: Vec<Request>,
}

impl App {
    /// Assemble the zones and register the one that starts out showing.
    pub fn new(startup: Startup, cfg: &crate::server::config::read::config::TuiConfig) -> Self {
        crate::ui::tui::theme::init_from_config(None, Some(&cfg.theme));
        let mut main = MainZone::default();
        main.set_usage_config(&cfg.usage);
        let mut history = crate::ui::tui::zone::main::history::zone::Config::default();
        history.focus_margin = cfg.render_margin.max(1);
        history.overscan = cfg.render_margin.max(1);
        history.todo_fold = cfg.todo_fold;
        main.set_history_config(&history);
        let mut resume = ResumeZone::default();
        let initial = startup.initial.unwrap_or(ZoneId::Main);
        if initial == ZoneId::Resume {
            resume.begin(startup.cwd);
        }
        Self {
            id: Uuid::new(),
            memo: Memo::default(),
            session: None,
            ownership: Ownership::new(initial),
            main,
            resume,
            tree: TreeZone::default(),
            carried: Vec::new(),
        }
    }

    pub fn id(&self) -> Uuid {
        self.id
    }

    pub fn born(&self) -> Uuid {
        self.id
    }

    pub fn died(&self) -> Uuid {
        self.id
    }

    /// `cherishYourWords` for the tree; false keeps Enter as rewind.
    pub fn set_tree_config(&mut self, cherish: bool) {
        self.tree.set_config(cherish);
    }

    /// Which zone shows, or `None` once nobody took over.
    pub fn showing(&self) -> Option<ZoneId> {
        self.ownership.showing()
    }

    /// Broadcast an event to the zone that shows.
    pub fn deliver_event(&mut self, ev: Event, ctx: &Ctx) {
        match self.ownership.showing() {
            Some(ZoneId::Main) => self.main.deliver_event(ev, ctx),
            Some(ZoneId::Resume) => self.resume.deliver_event(ev, ctx),
            Some(ZoneId::Tree) => self.tree.deliver_event(ev, ctx),
            None => {}
        }
    }

    /// Deliver an action; a handover it returns is executed here.
    pub fn deliver_action(&mut self, a: &Action, ctx: &Ctx) {
        let handover = match self.ownership.showing() {
            Some(ZoneId::Main) => self.main.deliver_action(a, ctx),
            Some(ZoneId::Resume) => self.resume.deliver_action(a, ctx),
            Some(ZoneId::Tree) => self.tree.deliver_action(a, ctx),
            None => None,
        };
        if let Some(h) = handover {
            self.hand_over(h, ctx);
        }
    }

    /// Draw a frame: the zone that shows draws itself.
    pub fn render(&mut self, ctx: &Ctx) -> Vec<ratatui::text::Line<'static>> {
        match self.ownership.showing() {
            Some(ZoneId::Main) => self.main.render(ctx),
            Some(ZoneId::Resume) => self.resume.render(ctx),
            Some(ZoneId::Tree) => self.tree.render(ctx),
            None => Vec::new(),
        }
    }

    /// Hardware cursor: only the chat zone's input asks for one.
    pub fn cursor(&self) -> Option<(u16, u16)> {
        match self.ownership.showing() {
            Some(ZoneId::Main) => self.main.cursor(),
            Some(ZoneId::Resume) | Some(ZoneId::Tree) => None,
            None => None,
        }
    }

    /// Take what the showing zone asks the loop to do.
    pub fn take_requests(&mut self) -> Vec<Request> {
        let mut out = std::mem::take(&mut self.carried);
        let showing = match self.ownership.showing() {
            Some(ZoneId::Main) => self.main.take_requests(),
            Some(ZoneId::Resume) => self.resume.take_requests(),
            Some(ZoneId::Tree) => self.tree.take_requests(),
            None => Vec::new(),
        };
        out.extend(showing);
        out
    }
}
