//! The session itself: lifecycle, the public calls, the caches, and the event exit.

use anyhow::Result;
use serde_json::Value;

use crate::ipc::endpoint::Endpoint;

use super::client::{Call, Client, Post, Request, Subscribe};
use super::content::{ContentCache, Direction};
use super::event::{Block, Event, MemoOp, MemoScope, ModelId, Retention};
use super::ingest;
use super::state::State;
use super::structure::Structure;

/// How many blocks one side read asks for.
const SIDE: i64 = 64;

/// The makings of a session whose row does not exist yet.
struct Draft {
    cwd: String,
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
}

impl Session {
    /// Connect to the server and register. Who starts the server is not this layer's business.
    pub fn connect(endpoint: Endpoint, cwd: String) -> Result<Session> {
        let client = Client::connect(endpoint, viewer())?;
        let mut session = Session {
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
        };
        session.client.queue(Request::Call(Call::Budget));
        Ok(session)
    }

    /// The makings of the next session this front end creates.
    pub fn set_draft(
        &mut self,
        cwd: String,
        model: Option<ModelId>,
        name: Option<String>,
        profile: Option<String>,
    ) {
        self.draft = Draft {
            cwd,
            model,
            name,
            profile,
        };
    }

    /// Hand the front end everything the connection produced since the last call.
    pub fn forward(&mut self) -> Vec<Event> {
        for arrival in self.client.pump() {
            ingest::apply(
                arrival,
                &mut self.state,
                &mut self.structure,
                &mut self.content,
            );
        }
        self.state.drain()
    }

    /// Record the session the front end is on. The watch is a subscription of its own.
    pub fn attach(&mut self, id: i64) {
        self.state.attach(id);
        self.structure.clear();
        self.content.clear();
    }

    /// Leave the session: the watch goes with the next `subscribe_watch`, the caches go now.
    pub fn detach(&mut self) {
        self.client.clear();
        self.state.detach();
        self.structure.clear();
        self.content.clear();
    }

    /// Say goodbye and let the connection go: everything queued leaves, then `bye`.
    pub fn bye(&mut self) {
        self.client.queue(Request::Bye);
        let _ = self.client.pump();
    }

    /// The session it is on; `None` = nothing attached.
    pub fn attached(&self) -> Option<i64> {
        self.state.attached()
    }

    pub fn busy(&self) -> bool {
        self.state.busy()
    }

    pub fn refused(&self) -> bool {
        self.state.refused()
    }

    /// The structure as it stands: a frozen copy the caller owns.
    pub fn local_struct_snap(&self) -> Structure {
        self.structure.clone()
    }

    /// The raw content cache: landed blocks, ascending by id.
    pub fn landed(&self) -> impl Iterator<Item = &Block> {
        self.content.landed()
    }

    /// Re-read the structure and the newest blocks, unless the row was just created here.
    pub fn call_init(&mut self, session: i64) {
        if self.state.attached() != Some(session) {
            return;
        }
        if self.state.init() == Some(session) {
            return;
        }
        if self.state.take_new_born() {
            return;
        }
        self.state.set_init(Some(session));
        self.client.queue(Request::Call(Call::Init { session }));
    }

    /// One round's whole interval set.
    pub fn call_turn(&mut self, turn: i64) {
        let Some(session) = self.state.attached() else {
            return;
        };
        let Some(ranges) = self.structure.ranges(turn) else {
            return;
        };
        let ranges: Vec<[i64; 2]> = ranges
            .iter()
            .map(|range| [*range.start(), *range.end()])
            .collect();
        if ranges.is_empty() {
            return;
        }
        self.client
            .queue(Request::Call(Call::Blocks { session, ranges }));
    }

    /// One side of the cache; `num` is the edge block.
    pub fn call_side(&mut self, num: i64, direction: Direction) {
        let Some(session) = self.state.attached() else {
            return;
        };
        let Some((lo, hi)) = self.structure.interval(num) else {
            return;
        };
        let range = match direction {
            Direction::Older => [lo.max(num - SIDE + 1), num],
            Direction::Newer => [num, hi.min(num + SIDE - 1)],
        };
        self.client.queue(Request::Call(Call::Blocks {
            session,
            ranges: vec![range],
        }));
    }

    pub fn call_meta(&mut self) {
        if let Some(session) = self.state.attached() {
            self.client.queue(Request::Call(Call::Meta { session }));
        }
    }

    pub fn call_usage(&mut self) {
        if let Some(session) = self.state.attached() {
            self.client.queue(Request::Call(Call::Usage { session }));
        }
    }

    pub fn call_todo(&mut self) {
        if let Some(session) = self.state.attached() {
            self.client.queue(Request::Call(Call::Todo { session }));
        }
    }

    pub fn call_memo(&mut self, op: MemoOp, scope: MemoScope, name: Option<String>) {
        let session = self.state.attached();
        if scope == MemoScope::Current && session.is_none() {
            return;
        }
        self.client.queue(Request::Call(Call::Memo {
            session,
            op,
            scope,
            name,
        }));
    }

    pub fn call_children(&mut self) {
        if let Some(session) = self.state.attached() {
            self.client.queue(Request::Call(Call::Children { session }));
        }
    }

    pub fn call_config(&mut self, key: String) {
        self.client.queue(Request::Call(Call::Config { key }));
    }

    pub fn call_pools(&mut self) {
        self.client.queue(Request::Call(Call::Pools));
    }

    pub fn call_sessions(&mut self, under: Option<String>) {
        self.client.queue(Request::Call(Call::Sessions { under }));
    }

    /// Create the session the draft describes.
    pub fn post_init(&mut self) {
        self.client.queue(Request::Post(Post::Init {
            cwd: self.draft.cwd.clone(),
            name: self.draft.name.clone(),
            model: self.draft.model.clone(),
            profile: self.draft.profile.clone(),
        }));
    }

    /// The display name (`None` clears it).
    pub fn post_name(&mut self, name: Option<String>) {
        let Some(session) = self.state.attached() else {
            self.draft.name = name;
            return;
        };
        self.client
            .queue(Request::Post(Post::Name { session, name }));
    }

    /// The working directory the reader typed.
    pub fn post_cwd(&mut self, path: String) {
        if let Some(session) = self.state.attached() {
            self.client
                .queue(Request::Post(Post::Cwd { session, path }));
        }
    }

    /// The models the session runs on.
    pub fn post_switch(&mut self, model: ModelId) {
        let Some(session) = self.state.attached() else {
            self.draft.model = Some(model);
            return;
        };
        self.client
            .queue(Request::Post(Post::Switch { session, model }));
    }

    pub fn post_delete(&mut self, session: i64) {
        self.client.queue(Request::Post(Post::Delete { session }));
    }

    pub fn post_interrupt(&mut self) {
        if let Some(session) = self.state.attached() {
            self.client
                .queue(Request::Post(Post::Interrupt { session }));
        }
    }

    pub fn post_rewind(&mut self, turn: i64, block: i64) {
        if let Some(session) = self.state.attached() {
            self.client.queue(Request::Post(Post::Rewind {
                session,
                turn,
                block,
            }));
        }
    }

    /// `/model` — the configured default; it takes effect on the next start.
    pub fn post_default_model(&mut self, model: ModelId) {
        self.client
            .queue(Request::Post(Post::DefaultModel { model }));
    }

    /// Follow the session's stream; a second call for the same session is a no-op.
    pub fn subscribe_watch(&mut self, session: i64) {
        if self.state.watching() == Some(session) {
            return;
        }
        self.state.set_watching(Some(session));
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

    pub fn subscribe_compact(
        &mut self,
        session: i64,
        retention: Option<Retention>,
        focus: Option<String>,
    ) {
        self.client.queue(Request::Subscribe(Subscribe::Compact {
            session,
            retention,
            focus,
        }));
    }
}

/// One identity for this front end instance.
fn viewer() -> Value {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Value::String(format!("{}-{now}", std::process::id()))
}
