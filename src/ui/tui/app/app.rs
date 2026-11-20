// The app: the registry, the display ledger, and the one post box between the zones and the session.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Instant;

use ratatui::text::Line;

use super::startup::Startup;
use crate::ui::session::Session;
use crate::ui::tui::action::Action;
use crate::ui::tui::theme::Palette;
use crate::ui::tui::zone::{
    Frame, Kind, Local, Mail, Memo, Outcome, Shared, TermSize, Zone, ZoneId, matches,
};

// One thing the loop hands the app.
pub enum Input {
    // The user did something.
    Action(Action),

    // The terminal said something.
    Local(Frame),
}

// The whole front end's state: the session, the zones, and who shows.
pub struct App {
    pub(super) session: Session,
    pub(super) zones: Vec<Box<dyn Zone>>,
    pub(super) owner: Option<ZoneId>,
    pub(super) memo: Memo,
    size: TermSize,
    pub(super) attached: Option<i64>,

    // The post box, the one color and the draft the zones share with this file.
    pub(super) shared: Shared,

    // The session each zone has last been told about; `None` = never told.
    pub(super) told: Vec<Option<Option<i64>>>,

    // Text waiting for the loop to write it to the clipboard.
    copy: Vec<String>,

    // Who wrote every letter still owed an answer, in the order they left; `None` = this file.
    pub(super) asking: VecDeque<Option<ZoneId>>,

    // What the last draw showed: the owner's revision, and when it was drawn.
    drawn: Option<(u64, Instant)>,
    finished: bool,

    // The directory a draft starts in when no zone has one yet.
    start_cwd: PathBuf,
}

impl App {
    // Assemble the front end: the shared surface, the zones, then the first screen.
    pub fn boot(
        session: Session,
        size: TermSize,
        startup: Startup,
        mut zones: Vec<Box<dyn Zone>>,
    ) -> App {
        let shared = Shared {
            mail: Mail::default(),
            palette: Palette::default(),
        };
        for (at, zone) in zones.iter_mut().enumerate() {
            zone.bind(shared.at(ZoneId::of_index(at)));
        }
        let told = vec![None; zones.len()];
        let mut app = App {
            session,
            zones,
            owner: Some(startup.initial.unwrap_or(ZoneId::Main)),
            memo: Memo::default(),
            size,
            attached: None,
            shared,
            told,
            copy: Vec::new(),
            asking: VecDeque::new(),
            drawn: None,
            finished: false,
            start_cwd: startup.cwd.clone(),
        };
        app.reset_memo();
        if let Some(id) = startup.session {
            app.attach(id);
        }
        if let Some(model) = &startup.model {
            app.memo.write("model", model);
        }
        app.show();
        app.flush();
        app
    }

    // Nobody shows: the loop ends.
    pub fn finished(&self) -> bool {
        self.finished
    }

    // Take what the connection produced: an answer to whoever wrote it, a push to whoever shows.
    pub fn pump(&mut self) {
        for frame in self.session.receive() {
            self.created(&frame);
            self.route(&frame);
        }
        self.flush();
    }

    // One frame's side effects: the zone that shows settles, and what it asks leaves now.
    pub fn tick(&mut self) {
        let Some(owner) = self.owner else {
            return;
        };
        let locals = self.zones[owner.index()].tick();
        self.land_all(locals);
        self.flush();
    }

    // The rows to draw now; `None` = nothing changed and no animation is due.
    pub fn frame(&mut self, now: Instant) -> Option<Vec<Line<'static>>> {
        let owner = self.owner?;
        let zone = &mut self.zones[owner.index()];
        let rev = zone.revision();

        let due = match self.drawn {
            None => true,
            Some((drawn, at)) => {
                drawn != rev
                    || zone
                        .anim()
                        .is_some_and(|period| now.saturating_duration_since(at) >= period)
            }
        };
        if !due {
            return None;
        }
        let lines = zone.render();
        self.drawn = Some((zone.revision(), now));
        Some(lines)
    }

    // One input from the loop.
    pub fn input(&mut self, input: Input) {
        match input {
            Input::Action(action) => self.action(&action),
            Input::Local(frame) => {
                if frame.name == "resized"
                    && let Some(size) = TermSize::of(&frame.payload)
                {
                    self.size = size;
                }
                self.deliver(&frame);
            }
        }
        self.flush();
    }

    // Where the hardware cursor goes.
    pub fn cursor(&self) -> Option<(u16, u16)> {
        match self.owner {
            Some(zone) => self.zones[zone.index()].cursor(),
            None => None,
        }
    }

    // Say goodbye and let the connection go.
    pub fn bye(&mut self) {
        self.session.close();
    }

    // Take the text the loop still owes the clipboard.
    pub fn take_copy(&mut self) -> Vec<String> {
        std::mem::take(&mut self.copy)
    }

    // A reply goes to the zone that wrote the letter; a push to the zone that shows.
    fn route(&mut self, frame: &Frame) {
        if frame.kind == "stream" || frame.kind == "local" {
            self.deliver(frame);
            return;
        }
        let Some(Some(to)) = self.asking.pop_front() else {
            return;
        };
        let locals = self.zones[to.index()].event(frame);
        self.land_all(locals);
    }

    fn deliver(&mut self, frame: &Frame) {
        let Some(owner) = self.owner else {
            return;
        };
        let index = owner.index();
        let declared = frame.kind == "local"
            || Kind::of(&frame.kind, &frame.name)
                .is_some_and(|kind| self.zones[index].kinds().contains(&kind));
        if !declared {
            return;
        }
        let locals = self.zones[index].event(frame);
        self.land_all(locals);
    }

    fn action(&mut self, action: &Action) {
        let Some(owner) = self.owner else {
            return;
        };
        if !self.zones[owner.index()]
            .keys()
            .iter()
            .any(|held| matches(held, action))
        {
            return;
        }
        let (outcome, locals) = self.zones[owner.index()].action(action);
        self.land_all(locals);
        self.settle(outcome);
    }

    // The zone that shows has been started, or is staying; carry the outcome.
    fn settle(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Stay => {}
            Outcome::Quit => {
                self.owner = None;
                self.finished = true;
            }
            Outcome::Show { to, memo } => {
                for (alias, content) in memo {
                    self.memo.write(&alias, &content);
                }
                self.owner = Some(to);
                self.drawn = None;
                self.tell();
                let locals = self.zones[to.index()].start(self.size, &self.memo);
                self.land_all(locals);
            }
        }
    }

    // Give the screen to the first zone.
    fn show(&mut self) {
        let Some(owner) = self.owner else {
            return;
        };
        let locals = self.zones[owner.index()].start(self.size, &self.memo);
        self.land_all(locals);
    }

    pub(super) fn land_all(&mut self, locals: Vec<Local>) {
        for local in locals {
            self.land(local);
        }
    }

    // Every letter the zones posted leaves now; its writer is remembered for the answer.
    pub(super) fn flush(&mut self) {
        for (from, frame) in self.shared.mail.drain() {
            self.session.send(frame);
            self.asking.push_back(Some(from));
        }
    }

    // The asks that never leave this process.
    fn land(&mut self, local: Local) {
        match local {
            Local::Attach(id) => self.attach(id),
            Local::Detach => self.detach(),
            Local::Quit => self.settle(Outcome::Quit),
            Local::Copy(text) => self.copy.push(text),
        }
    }

    // The row a draft's opening letter created: enter it, and the draft is spent.
    fn created(&mut self, frame: &Frame) {
        if frame.kind != "post" || frame.name != "init" {
            return;
        }
        if self.attached.is_some() {
            return;
        }
        if frame.payload.get("error").is_some() {
            return;
        }
        let Some(id) = frame.payload.get("session").and_then(|id| id.as_i64()) else {
            return;
        };
        if let Some(landed) = frame.payload.get("cwd").and_then(|cwd| cwd.as_str()) {
            self.zones[ZoneId::Main.index()].draft_moved(landed);
        }
        self.attach(id);
    }

    // Drop what one zone left for the next; the launch directory is always kept.
    pub(super) fn reset_memo(&mut self) {
        self.memo.clear();
        let cwd = self.zones[ZoneId::Main.index()]
            .draft_cwd()
            .unwrap_or_else(|| self.start_cwd.to_string_lossy().into_owned());
        self.memo.write("cwd", &cwd);
    }
}
