//! The app: the registry, the display ledger, and the only exit to the session.

use std::path::PathBuf;

use ratatui::text::Line;
use serde_json::{Value, json};

use super::startup::Startup;
use crate::ui::session::Session;
use crate::ui::tui::action::Action;
use crate::ui::tui::theme::Accent;
use crate::ui::tui::zone::{
    Frame, Kind, Memo, ModelId, Outcome, Request, Round, TermSize, Turns, Zone, ZoneId, matches,
};

/// One thing the loop hands the app.
pub enum Input {
    /// The user did something.
    Action(Action),
    /// The terminal or the session said something.
    Local(Frame),
}

/// The whole front end's state: the session, the zones, and who shows.
pub struct App {
    pub(super) session: Session,
    pub(super) zones: Vec<Box<dyn Zone>>,
    pub(super) owner: Option<ZoneId>,
    pub(super) memo: Memo,
    size: TermSize,
    pub(super) attached: Option<i64>,
    /// The one color the input zone and the running cards wear.
    accent: Accent,
    /// The directory a new session is created in.
    cwd: PathBuf,
    /// `--model`: the model a new session is created with; `None` = the configured default.
    model: Option<ModelId>,
    /// A message sent with no session yet: it goes out once the row exists.
    waiting: Option<Value>,
    /// The session each zone has last been told about; a zone hears it once.
    pub(super) told: Vec<Option<i64>>,
    finished: bool,
}

impl App {
    /// Assemble the front end: ask the palette, enter the session, start the first zone.
    pub fn boot(
        session: Session,
        size: TermSize,
        startup: Startup,
        zones: Vec<Box<dyn Zone>>,
    ) -> App {
        let told = vec![None; zones.len()];
        let mut app = App {
            session,
            zones,
            owner: Some(startup.initial.unwrap_or(ZoneId::Main)),
            memo: Memo::default(),
            size,
            attached: None,
            accent: Accent::pick(),
            cwd: startup.cwd,
            model: startup.model,
            waiting: None,
            told,
            finished: false,
        };
        app.reset_memo();
        if let Some(id) = startup.session {
            app.attach(id);
        }
        app.show();
        app
    }

    /// Nobody shows: the loop ends.
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// Take everything the connection produced and hand it to the zone that declared it.
    pub fn pump(&mut self) {
        for packet in self.session.forward() {
            let frame = Frame::new(packet.kind, packet.name, packet.content);
            if frame.class == "post" && frame.name == "init" {
                self.created(&frame);
            }
            if frame.class == "post" && frame.name == "name" {
                self.accent = self.accent.pick_other();
                self.announce_accent();
            }
            self.deliver(&frame);
        }
        let live = self.session.attached();
        if live != self.attached {
            match live {
                Some(id) => self.attach(id),
                None => self.detach(),
            }
        }
    }

    /// One input from the loop.
    pub fn input(&mut self, input: Input) {
        match input {
            Input::Action(action) => self.action(&action),
            Input::Local(frame) => {
                if frame.name == "resized"
                    && let Some(size) = TermSize::of(&frame.content)
                {
                    self.size = size;
                }
                self.deliver(&frame);
            }
        }
    }

    /// This frame's rows, from the zone that shows.
    pub fn render(&mut self) -> Vec<Line<'static>> {
        match self.owner {
            Some(zone) => self.zones[zone.index()].render(),
            None => Vec::new(),
        }
    }

    /// Where the hardware cursor goes.
    pub fn cursor(&self) -> Option<(u16, u16)> {
        match self.owner {
            Some(zone) => self.zones[zone.index()].cursor(),
            None => None,
        }
    }

    /// Say goodbye and let the connection go.
    pub fn bye(&mut self) {
        self.session.bye();
    }

    fn deliver(&mut self, frame: &Frame) {
        let Some(owner) = self.owner else {
            return;
        };
        let index = owner.index();
        let declared = frame.is_local()
            || Kind::of(&frame.class, &frame.name)
                .is_some_and(|kind| self.zones[index].kinds().contains(&kind));
        if !declared {
            return;
        }
        let requests = self.zones[index].event(frame);
        self.land_all(requests);
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
        let (outcome, requests) = self.zones[owner.index()].action(action);
        self.land_all(requests);
        self.settle(outcome);
    }

    /// The zone that shows has been started, or is staying; carry the outcome.
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
                self.tell();
                let requests = self.zones[to.index()].start(self.size, &self.memo);
                self.land_all(requests);
                self.announce_accent();
            }
        }
    }

    /// Give the screen to the first zone.
    fn show(&mut self) {
        let Some(owner) = self.owner else {
            return;
        };
        let requests = self.zones[owner.index()].start(self.size, &self.memo);
        self.land_all(requests);
        self.announce_accent();
    }

    fn structure(&mut self) {
        let Some(owner) = self.owner else {
            return;
        };
        let snapshot = self.session.structure();
        let rounds: Vec<Round> = snapshot
            .turns()
            .iter()
            .map(|turn| Round::new(turn.id, turn.ranges.clone()))
            .collect();
        // One snapshot answers the ask; a nested ask would only loop.
        let requests: Vec<Request> = self.zones[owner.index()]
            .structure(&Turns::of(rounds))
            .into_iter()
            .filter(|request| !matches!(request, Request::Structure))
            .collect();
        self.land_all(requests);
    }

    pub(super) fn land_all(&mut self, requests: Vec<Request>) {
        for request in requests {
            self.land(request);
        }
    }

    /// The only exit to the session.
    fn land(&mut self, request: Request) {
        match request {
            Request::Send {
                kind,
                name,
                content,
                answer,
            } => {
                // A message with no session yet: the row comes first, the text after it.
                if kind == Kind::Subscribe && name == "submit" && self.session.attached().is_none()
                {
                    self.waiting = Some(content);
                    self.create(None);
                    return;
                }
                let content = with_session(content, self.session.attached());
                self.session.send(kind.as_str(), name, content, answer);
            }
            Request::Attach(id) => self.attach(id),
            Request::Detach => self.detach(),
            Request::Structure => self.structure(),
            Request::Create { profile } => self.profile(profile),
            Request::Quit => self.settle(Outcome::Quit),
        }
    }

    /// A profile before the first message: open the row with it, or change the empty one.
    fn profile(&mut self, profile: Option<String>) {
        match (self.session.attached(), profile) {
            (Some(_), Some(name)) => {
                self.land(Request::post("profile", json!({"name": name})));
            }
            (Some(_), None) => {}
            (None, profile) => self.create(profile),
        }
    }

    /// Drop what one zone left for the next; the launch directory is always kept.
    pub(super) fn reset_memo(&mut self) {
        self.memo.clear();
        self.memo.write("cwd", &self.cwd.to_string_lossy());
    }

    /// Create the session a waiting message needs; a first command may name its profile.
    fn create(&mut self, profile: Option<String>) {
        let model = self.model.as_ref().map(|id| {
            json!({
                "provider": id.provider,
                "model": id.model,
            })
        });
        let content = json!({
            "cwd": self.cwd.to_string_lossy(),
            "name": Value::Null,
            "model": model,
            "profile": profile,
        });
        self.session.send("post", "init", content, true);
    }

    /// The row exists: enter it, then send the message that was waiting for it.
    fn created(&mut self, frame: &Frame) {
        if self.attached.is_some() {
            return;
        }
        if frame.content.get("error").is_some() {
            self.waiting = None;
            return;
        }
        let Some(id) = frame.content.get("session").and_then(Value::as_i64) else {
            return;
        };
        self.attach(id);
        if let Some(content) = self.waiting.take() {
            self.land(Request::Send {
                kind: Kind::Subscribe,
                name: "submit",
                content,
                answer: true,
            });
        }
    }

    /// Hand the current accent to whatever shows.
    fn announce_accent(&mut self) {
        let frame = Frame::new("local", "accent", json!({"color": self.accent.hex()}));
        self.deliver(&frame);
    }
}

/// Fill the session into one frame's content, unless the zone named one.
fn with_session(content: Value, session: Option<i64>) -> Value {
    let Value::Object(mut map) = content else {
        return content;
    };
    if !map.contains_key("session")
        && let Some(session) = session
    {
        map.insert("session".to_string(), json!(session));
    }
    Value::Object(map)
}
