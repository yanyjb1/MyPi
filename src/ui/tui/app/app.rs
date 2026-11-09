//! The app: the registry, the display ledger, and the only exit to the session.

use std::path::PathBuf;

use ratatui::text::Line;
use serde_json::{Value, json};

use super::startup::Startup;
use super::wire;
use crate::ui::session::Session;
use crate::ui::tui::action::Action;
use crate::ui::tui::theme::Accent;
use crate::ui::tui::zone::{
    Frame, Kind, Memo, ModelId, Outcome, Request, Round, TermSize, Turns, Zone, ZoneId, matches,
};

/// A usable launch directory: the home directory and its parent are not workspaces, so a launch
/// there lands in the temp dir instead.
fn workspace(dir: PathBuf) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let barred = home.as_ref().is_some_and(|home| {
        dir == *home || home.parent().is_some_and(|parent| dir.as_path() == parent)
    });
    if barred { std::env::temp_dir() } else { dir }
}

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
    pub(super) cwd: PathBuf,
    /// `--model`: the model a new session is created with; `None` = the configured default.
    model: Option<ModelId>,
    /// `/profile` before any session: the name travels with the row that opens.
    pub(super) profile: Option<String>,
    /// `/name` before any session: the name travels with the row that opens.
    pub(super) name: Option<String>,
    /// The message whose opening `init` is in flight; it goes out as soon as the row answers.
    opening: Option<String>,
    /// The session each zone has last been told about; `None` = never told.
    pub(super) told: Vec<Option<Option<i64>>>,
    /// Text waiting for the loop to write it to the clipboard.
    copy: Vec<String>,
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
            cwd: workspace(startup.cwd),
            model: startup.model,
            profile: None,
            name: None,
            opening: None,
            told,
            copy: Vec::new(),
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

    /// Take the text the loop still owes the clipboard.
    pub fn take_copy(&mut self) -> Vec<String> {
        std::mem::take(&mut self.copy)
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
            Request::Attach(id) => self.attach(id),
            Request::Detach => self.detach(),
            Request::Structure => self.structure(),
            Request::Quit => self.settle(Outcome::Quit),
            Request::PickProfile(profile) => self.pick_profile(profile),
            Request::Copy(text) => self.copy.push(text),
            Request::Submit(text) => self.submit(text),
            Request::SetCwd(path) => self.set_cwd(path),
            Request::Rename(name) => self.rename(name),
            Request::SwitchModel(id) if self.session.attached().is_none() => {
                self.model = Some(id);
                let address = self
                    .model
                    .as_ref()
                    .map(|id| format!("{}:{}", id.provider, id.model))
                    .unwrap_or_default();
                let frame = Frame::new("local", "switch", json!({"address": address}));
                self.deliver(&frame);
            }
            other => wire::land(&mut self.session, other),
        }
    }

    /// A move lands on the session when there is one; a draft has only its launch directory,
    /// which is what the row will be created with.
    fn set_cwd(&mut self, path: String) {
        if self.session.attached().is_some() {
            self.session.set_cwd(&path);
            return;
        }
        let target = if std::path::Path::new(&path).is_absolute() {
            PathBuf::from(&path)
        } else {
            self.cwd.join(&path)
        };
        let target = target.canonicalize().unwrap_or(target);
        self.cwd = workspace(target);
        let frame = Frame::new("local", "cwd", json!({"cwd": self.cwd.to_string_lossy()}));
        self.deliver(&frame);
    }

    /// `/name` lands on the session when there is one; a draft keeps it for the row.
    fn rename(&mut self, name: Option<String>) {
        if self.session.attached().is_some() {
            self.session.rename(name.as_deref());
            return;
        }
        self.name = name;
        let frame = Frame::new("local", "name", json!({}));
        self.deliver(&frame);
    }

    /// A message: the row comes first when there is none, then the text, then its echo.
    fn submit(&mut self, text: String) {
        if self.session.attached().is_none() {
            self.opening = Some(text);
            self.create();
            return;
        }
        self.session.submit(&text);
        self.echo(&text);
    }

    /// A message just went out: the transcript draws it before the round answers.
    fn echo(&mut self, text: &str) {
        let frame = Frame::new("local", "echo", json!({"text": text}));
        self.deliver(&frame);
    }

    /// `/profile` before any session: the name stays in this front end until the row opens.
    fn pick_profile(&mut self, profile: Option<String>) {
        if self.session.attached().is_none() {
            self.profile = profile;
        }
    }

    /// Drop what one zone left for the next; the launch directory is always kept.
    pub(super) fn reset_memo(&mut self) {
        self.memo.clear();
        self.memo.write("cwd", &self.cwd.to_string_lossy());
    }

    /// Create the row an opening message needs; a profile picked earlier travels with it.
    fn create(&mut self) {
        let model = self.model.as_ref().map(|id| {
            json!({
                "provider": id.provider,
                "model": id.model,
            })
        });
        let content = json!({
            "cwd": self.cwd.to_string_lossy(),
            "name": self.name.clone(),
            "model": model,
            "profile": self.profile.clone(),
        });
        self.session.send("post", "init", content, true);
    }

    /// The row exists: enter it, then send the message that opened it.
    fn created(&mut self, frame: &Frame) {
        if self.attached.is_some() {
            return;
        }
        if frame.content.get("error").is_some() {
            self.opening = None;
            return;
        }
        let Some(id) = frame.content.get("session").and_then(Value::as_i64) else {
            return;
        };
        self.attach(id);
        if let Some(text) = self.opening.take() {
            self.session.submit(&text);
            self.echo(&text);
        }
    }

    /// Hand the current accent to whatever shows.
    fn announce_accent(&mut self) {
        let frame = Frame::new("local", "accent", json!({"color": self.accent.hex()}));
        self.deliver(&frame);
    }
}
