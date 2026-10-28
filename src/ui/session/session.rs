//! The session itself: lifecycle, the public calls, the caches, and the event exit.

use std::path::PathBuf;

use anyhow::Result;

use crate::ipc::contract::{
    Call, MemoOp, MemoScope, MemoWrite, ModelId, Post, PostDone, Retention, ServerAsk, ServerWrite,
    SessionAsk, SessionWrite, Subscribe, ToolAsk, ToolWrite, UsageLang, Viewer,
};
use crate::ipc::endpoint::Endpoint;

use super::client::{Arrival, Client, Request};
use super::content::{ContentCache, Direction};
use super::ingest;
use super::state::State;
use super::structure::Structure;

/// The knobs a front end reads out of its configuration, handed in once at startup.
pub struct Switches {
    /// `config.ui.session.askedEachTime` — how many blocks one side read asks for.
    pub asked: usize,
}

impl Default for Switches {
    fn default() -> Self {
        Self { asked: 64 }
    }
}

/// The makings of a session whose row does not exist yet.
struct Draft {
    cwd: PathBuf,
    model: Option<ModelId>,
    name: Option<String>,
    profile: Option<String>,
}

pub struct Session {
    client: Client,
    state: State,
    structure: Structure,
    content: ContentCache,
    draft: Draft,
    /// The session whose first frame is in flight; one at a time.
    init_in_flight: Option<i64>,
    /// The limit the next side read asks for; `local_settle` restores it.
    budget: usize,
    /// One read's block count (`config.ui.session.askedEachTime`).
    asked: usize,
}

impl Session {
    /// Connect to the server and register. Who starts the server is not this layer's business.
    pub fn connect(endpoint: Endpoint, cwd: PathBuf, switches: Switches) -> Result<Session> {
        let client = Client::connect(endpoint, viewer())?;
        Ok(Session {
            client,
            state: State::new(),
            structure: Structure::default(),
            content: ContentCache::default(),
            draft: Draft {
                cwd,
                model: None,
                name: None,
                profile: None,
            },
            init_in_flight: None,
            budget: switches.asked.max(1),
            asked: switches.asked.max(1),
        })
    }

    /// Apply configuration a front end has read: the request sizing follows it.
    pub fn apply_ui_config(&mut self, switches: Switches) {
        self.asked = switches.asked.max(1);
        self.budget = self.asked;
    }

    /// Hand the front end everything the connection produced since the last call.
    pub fn forward(&mut self) -> Vec<Arrival> {
        self.pump();
        self.state.drain()
    }

    /// The upstream loop: send what is queued, take what came back, write it into the caches.
    fn pump(&mut self) {
        for arrival in self.client.pump() {
            let created = match &arrival {
                Arrival::Done(PostDone::Created(id)) => Some(*id),
                _ => None,
            };
            if let Some(id) = created {
                self.detach();
                self.attach(id);
            }
            let init = match &arrival {
                Arrival::Answer {
                    call:
                        Call::Session {
                            session,
                            ask: SessionAsk::Init { .. },
                        },
                    ..
                }
                | Arrival::Failed {
                    call:
                        Some(Call::Session {
                            session,
                            ask: SessionAsk::Init { .. },
                        }),
                    ..
                } => Some(*session),
                _ => None,
            };
            if let Some(session) = init
                && self.init_in_flight == Some(session)
            {
                self.init_in_flight = None;
            }
            ingest::apply(
                arrival,
                &mut self.state,
                &mut self.structure,
                &mut self.content,
            );
        }
    }

    /// Record the session the front end is on. The watch is a subscription of its own.
    pub fn attach(&mut self, id: i64) {
        self.state.attach(id);
        self.structure.clear();
        self.content.clear();
        self.init_in_flight = None;
    }

    /// Leave the session: the watch goes, and so does everything about it.
    pub fn detach(&mut self) {
        self.client.clear();
        self.client.queue(Request::Unwatch);
        self.state.detach();
        self.structure.clear();
        self.content.clear();
        self.init_in_flight = None;
        self.budget = self.asked;
    }

    pub fn bye(&mut self) {
        self.client.queue(Request::Bye);
    }

    /// The structure as it stands: a frozen copy the caller owns.
    pub fn local_struct_snap(&self) -> Structure {
        self.structure.clone()
    }

    /// The jump stopped: the next side read starts from the full budget again.
    pub fn local_settle(&mut self) {
        self.budget = self.asked;
    }

    /// Re-read the structure and the newest blocks, unless the row was just created here.
    pub fn call_init(&mut self, session: i64, limit: usize) {
        if self.state.attached() != Some(session) {
            return;
        }
        if self.state.take_new_born() {
            return;
        }
        if self.init_in_flight == Some(session) {
            return;
        }
        self.init_in_flight = Some(session);
        self.call(Call::Session {
            session,
            ask: SessionAsk::Init { limit },
        });
    }

    /// One round's whole interval set.
    pub fn call_turn(&mut self, turn: i64) {
        let Some(session) = self.state.attached() else {
            return;
        };
        let Some(ranges) = self.structure.ranges(turn) else {
            return;
        };
        let ranges = ranges
            .iter()
            .map(|range| [*range.start(), *range.end()])
            .collect();
        self.budget = next_budget(self.budget);
        self.call(Call::Session {
            session,
            ask: SessionAsk::Blocks { ranges },
        });
    }

    /// One side of the cache; `num` is the edge block.
    pub fn call_side(&mut self, num: i64, direction: Direction) {
        let Some(session) = self.state.attached() else {
            return;
        };
        let limit = self.budget.max(1) as i64;
        let range = match direction {
            Direction::Older => {
                let first = self.structure.first_id().unwrap_or(1);
                [num.saturating_sub(limit - 1).max(first), num]
            }
            Direction::Newer => {
                let last = self.structure.last_id().unwrap_or(num);
                [num, num.saturating_add(limit - 1).min(last)]
            }
        };
        self.call(Call::Session {
            session,
            ask: SessionAsk::Blocks {
                ranges: vec![range],
            },
        });
    }

    pub fn call_meta(&mut self) {
        if let Some(session) = self.state.attached() {
            self.call(Call::Session {
                session,
                ask: SessionAsk::Meta,
            });
        }
    }

    pub fn call_usage(&mut self, lang: UsageLang) {
        if let Some(session) = self.state.attached() {
            self.call(Call::Session {
                session,
                ask: SessionAsk::Usage { lang },
            });
        }
    }

    pub fn call_todo(&mut self) {
        if let Some(session) = self.state.attached() {
            self.call(Call::Tool(ToolAsk::Todo { session }));
        }
    }

    pub fn call_memo(&mut self, op: MemoOp, scope: MemoScope, name: Option<String>) {
        self.call(Call::Tool(ToolAsk::Memo {
            session: self.state.attached(),
            op,
            scope,
            name,
        }));
    }

    pub fn call_children(&mut self) {
        if let Some(session) = self.state.attached() {
            self.call(Call::Tool(ToolAsk::Children { session }));
        }
    }

    pub fn call_config(&mut self, key: String) {
        self.call(Call::Server(ServerAsk::Config { key }));
    }

    pub fn call_pools(&mut self) {
        self.call(Call::Server(ServerAsk::Pools));
    }

    pub fn call_sessions(&mut self, under: Option<PathBuf>) {
        self.call(Call::Server(ServerAsk::Sessions { under }));
    }

    /// Create the session the draft describes.
    pub fn post_init(&mut self) {
        self.client.queue(Request::Post(Post::Init {
            cwd: self.draft.cwd.clone(),
            model: self.draft.model.clone(),
            name: self.draft.name.clone(),
            profile: self.draft.profile.clone(),
        }));
    }

    /// `/name` — the display name (`None` clears it).
    pub fn post_name(&mut self, name: Option<String>) {
        let Some(session) = self.state.attached() else {
            self.draft.name = name;
            return;
        };
        self.post(SessionWrite::Name(name), session);
    }

    /// `/cdp` — migrate the working directory; the path is what the reader typed.
    pub fn post_cwd(&mut self, path: String) {
        if let Some(session) = self.state.attached() {
            self.post(SessionWrite::Cwd(path), session);
        }
    }

    pub fn post_switch(&mut self, id: ModelId) {
        let Some(session) = self.state.attached() else {
            self.draft.model = Some(id);
            return;
        };
        self.post(SessionWrite::Switch(id), session);
    }

    pub fn post_profile(&mut self, profile: String) {
        let Some(session) = self.state.attached() else {
            self.draft.profile = Some(profile);
            return;
        };
        self.post(SessionWrite::Profile(profile), session);
    }

    pub fn post_todo_on(&mut self, call_id: String) {
        if let Some(session) = self.state.attached() {
            self.post(SessionWrite::TodoOn { call_id }, session);
        }
    }

    pub fn post_todo_off(&mut self, call_id: String) {
        if let Some(session) = self.state.attached() {
            self.post(SessionWrite::TodoOff { call_id }, session);
        }
    }

    pub fn post_delete(&mut self, id: i64) {
        self.post(SessionWrite::Delete, id);
    }

    pub fn post_interrupt(&mut self) {
        if let Some(session) = self.state.attached() {
            self.post(SessionWrite::Interrupt, session);
        }
    }

    pub fn post_rewind(&mut self, turn: i64, block: i64) {
        if let Some(session) = self.state.attached() {
            self.post(SessionWrite::Rewind { turn, block }, session);
        }
    }

    /// `/model` — the configured default; it takes effect on the next start.
    pub fn post_default_model(&mut self, id: ModelId) {
        self.client
            .queue(Request::Post(Post::Server(ServerWrite::DefaultModel(id))));
    }

    pub fn post_reload(&mut self) {
        self.client
            .queue(Request::Post(Post::Server(ServerWrite::Reload)));
    }

    pub fn post_memo(&mut self, write: MemoWrite) {
        self.client
            .queue(Request::Post(Post::Tool(ToolWrite::Memo(write))));
    }

    /// Stop the server pushing this session.
    pub fn post_unwatch(&mut self) {
        if let Some(session) = self.state.attached() {
            self.post(SessionWrite::Unwatch, session);
        }
    }

    pub fn subscribe_watch(&mut self, session: i64) {
        self.client
            .queue(Request::Subscribe(Subscribe::Watch { session }));
    }

    pub fn subscribe_submit(
        &mut self,
        session: i64,
        prefix: Option<String>,
        text: String,
        suffix: Option<String>,
    ) {
        self.client.queue(Request::Subscribe(Subscribe::Submit {
            session,
            prefix,
            text,
            suffix,
        }));
    }

    pub fn subscribe_compact(&mut self, session: i64, retention: Retention, focus: Option<String>) {
        self.client.queue(Request::Subscribe(Subscribe::Compact {
            session,
            retention,
            focus,
        }));
    }

    fn call(&mut self, call: Call) {
        self.client.queue(Request::Call(call));
    }

    fn post(&mut self, write: SessionWrite, session: i64) {
        self.client
            .queue(Request::Post(Post::Session { session, write }));
    }
}

/// One identity for this front end instance.
fn viewer() -> Viewer {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Viewer(format!("{}-{now}", std::process::id()))
}

/// The next side read asks for less until the reader settles.
fn next_budget(current: usize) -> usize {
    if current > 8 { 8 } else { (current / 2).max(4) }
}
