//! session — the layer every front end sits on: the backend, the state, the raw cache.
//! Design: `README.md`.

mod client;
mod command;
mod ingest;
mod range;
mod shape;
mod state;
mod window;

pub use range::RangeCache;
pub use shape::Shapes;
pub use state::{Level, Life, Notice, UsagePanel};
pub use window::{BlockRef, LiveBlock, LiveTail, ToolOutput, WindowBlock};

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::server::agent::tools::TodoPhase;
use crate::server::api::call::sessions::SessionRow;
use crate::server::api::subscribe::Retention;
use crate::server::api::types::{Call, CandidatePools, ModelId, Post, UsageLang, Viewer};
use crate::server::platform::ipc::Cost;
use crate::server::storage::{BlockKind, NewBlock};

use client::{Client, Request};
use ingest::Follow;
use state::State;
use window::Window;

/// The knobs a front end reads out of its configuration, handed in once at startup.
pub struct Switches {
    /// `config.ui.session.preloadCache` — how many blocks the content cache keeps.
    pub preload: usize,
    /// `config.ui.session.askedEachTime` — how many blocks one read asks for.
    pub asked: usize,
    /// `config.ui.placeholder.interrupted` — the words the optimistic interrupt row carries.
    pub interrupted: String,
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
    viewer: Viewer,
    state: State,
    window: Window,
    shapes: Shapes,
    range: RangeCache,
    draft: Draft,
    /// A message sent with no row yet: it goes out as soon as the id arrives.
    waiting: Option<String>,
    /// The session whose first frame is in flight; one at a time.
    init_in_flight: Option<i64>,
    /// The limit the next jump asks for; `Settled` restores it.
    jump_budget: usize,
    /// One read's block count (`config.ui.session.askedEachTime`).
    asked: usize,
}

impl Session {
    /// Connect to the server and register. Who starts the server is not this layer's business.
    pub fn connect(
        cost: Cost,
        path: &Path,
        cwd: PathBuf,
        switches: Switches,
        viewer: Viewer,
    ) -> Result<Session> {
        let client = Client::connect(cost, path, viewer)?;
        let mut window = Window::new(switches.preload);
        window.set_placeholder_text(switches.interrupted);
        Ok(Session {
            client,
            viewer,
            state: State::new(cwd.to_string_lossy().into_owned()),
            window,
            shapes: Shapes::default(),
            range: RangeCache::default(),
            draft: Draft {
                cwd,
                model: None,
                name: None,
                profile: None,
            },
            waiting: None,
            init_in_flight: None,
            jump_budget: switches.asked,
            asked: switches.asked,
        })
    }

    /// The id this front end registered with.
    pub fn viewer(&self) -> Viewer {
        self.viewer
    }

    /// Send what was queued and take what came back.
    pub fn pump(&mut self) {
        for arrival in self.client.pump() {
            let init = match &arrival {
                Arrival::Answer {
                    call: Call::Init { session, .. },
                    ..
                } => Some(*session),
                Arrival::Failed {
                    call: Some(Call::Init { session, .. }),
                    ..
                } => Some(*session),
                _ => None,
            };
            let follow = ingest::apply(
                arrival,
                &mut self.state,
                &mut self.window,
                &mut self.shapes,
                &mut self.range,
            );
            if let Some(session) = init
                && self.init_in_flight == Some(session)
            {
                self.init_in_flight = None;
            }
            self.follow(follow);
        }
    }

    fn follow(&mut self, follow: Follow) {
        match follow {
            Follow::Nothing => {}
            Follow::Created(id) => {
                self.enter(id);
                if let Some(text) = self.waiting.take() {
                    self.submit(text);
                }
            }
            Follow::Watch(id) => self.enter(id),
            Follow::Todo(id) => self.read(Call::Todo { session: id }),
            Follow::Shape { session, turn } => self.read(Call::Turn { session, turn }),
        }
    }

    /// Enter a session: watch it, take its structure and newest blocks, plus the facts beside it.
    fn enter(&mut self, id: i64) {
        self.state.attach(id);
        self.shapes.clear();
        self.range.clear();
        self.client.queue(Request::Watch(id));
        self.init_in_flight = Some(id);
        self.read(Call::Init {
            session: id,
            limit: self.asked,
        });
        self.read(Call::Meta { session: id });
        self.read(Call::Todo { session: id });
    }

    /// Re-read the structure and newest blocks, unless the row was just created here.
    pub fn init(&mut self, session: i64) {
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
        self.read(Call::Init {
            session,
            limit: self.asked,
        });
    }

    /// Jump to one round: the window is replaced by that round's blocks.
    pub fn jump(&mut self, turn: i64) {
        let Some(session) = self.state.attached() else {
            return;
        };
        self.jump_budget = next_budget(self.jump_budget);
        self.read(Call::Round { session, turn });
    }

    /// Fetch one side of the window; `num` is the edge block.
    pub fn fetch(&mut self, num: i64, older: bool) {
        let Some(session) = self.state.attached() else {
            return;
        };
        let turn = self.shapes.turn_of(num);
        if older {
            self.read(Call::Older {
                session,
                before: num,
                limit: self.asked,
                turn,
            });
        } else {
            self.read(Call::Newer {
                session,
                after: num,
                limit: self.asked,
                turn,
            });
        }
    }

    /// The jump stopped: the next one starts from the full budget again.
    pub fn settled(&mut self) {
        self.jump_budget = self.asked;
    }

    /// Read raw blocks for the tree.
    pub fn tree_range(&mut self, start: i64, end: i64) {
        let Some(session) = self.state.attached() else {
            return;
        };
        self.read(Call::Range {
            session,
            start,
            end,
        });
    }

    fn read(&mut self, call: Call) {
        self.client.queue(Request::Call(call));
    }

    /// The makings of a session with no row yet; the first message creates it from these.
    pub fn set_draft(
        &mut self,
        cwd: PathBuf,
        model: Option<ModelId>,
        name: Option<String>,
        profile: Option<String>,
    ) {
        self.state.set_cwd(cwd.to_string_lossy().into_owned());
        self.draft = Draft {
            cwd,
            model,
            name,
            profile,
        };
    }

    pub fn open_new(&mut self) {
        let draft = self.draft();
        self.client.queue(Request::Post(Post::Init {
            cwd: draft.cwd,
            model: draft.model,
            name: draft.name,
            profile: draft.profile,
        }));
    }

    fn draft(&self) -> Draft {
        Draft {
            cwd: self.draft.cwd.clone(),
            model: self.draft.model.clone(),
            name: self.draft.name.clone(),
            profile: self.draft.profile.clone(),
        }
    }

    pub fn attach(&mut self, id: i64) {
        self.follow(Follow::Watch(id));
    }

    /// Leave the session: the watch goes, and so does everything about it.
    pub fn detach(&mut self) {
        self.client.queue(Request::Unwatch);
        self.window.replace(Vec::new(), false);
        self.shapes.clear();
        self.range.clear();
        self.init_in_flight = None;
        self.waiting = None;
        self.jump_budget = self.asked;
        self.state.detach();
    }

    pub fn bye(&mut self) {
        self.client.queue(Request::Bye);
    }

    /// Send one message; with no row yet, create it first and send after its id.
    pub fn submit(&mut self, text: String) {
        let Some(session) = self.state.attached() else {
            self.waiting = Some(text);
            self.open_new();
            return;
        };
        self.state.set_busy(true);
        self.window.clear_placeholder();
        self.client.queue(Request::Submit {
            session,
            prefix: None,
            text,
            suffix: None,
        });
    }

    /// Kill the running round; the placeholder for it stands right away.
    pub fn interrupt(&mut self) {
        let Some(session) = self.state.attached() else {
            return;
        };
        self.state.set_busy(false);
        self.window.stand_placeholder();
        self.client
            .queue(Request::Post(Post::Interrupt { session }));
    }

    pub fn compact(&mut self, retention: Retention, focus: Option<String>) {
        let Some(session) = self.state.attached() else {
            return;
        };
        self.state.set_busy(true);
        self.client.queue(Request::Compact {
            session,
            retention,
            focus,
        });
    }

    /// Cut the conversation back to a block, and take the transcript again.
    pub fn rewind(&mut self, turn: i64, block: i64) {
        let Some(session) = self.state.attached() else {
            return;
        };
        self.client.queue(Request::Post(Post::Rewind {
            session,
            turn,
            block,
        }));
        // Everything past the cut is gone: the kept structure names rounds that no longer exist.
        self.shapes.clear();
        self.range.clear();
        self.init_in_flight = Some(session);
        self.read(Call::Init {
            session,
            limit: self.asked,
        });
    }

    /// `/name` — the display name (`None` clears it).
    pub fn rename(&mut self, name: Option<String>) {
        let Some(session) = self.state.attached() else {
            self.draft.name = name;
            return;
        };
        self.state.set_name(name.clone());
        self.client
            .queue(Request::Post(Post::Name { session, name }));
    }

    /// `/cdp` — migrate the working directory; the path is what the reader typed.
    pub fn set_cwd(&mut self, path: String) {
        let Some(session) = self.state.attached() else {
            return;
        };
        self.state.migrate(&path);
        self.client
            .queue(Request::Post(Post::Cwd { session, path }));
        self.read(Call::Meta { session });
    }

    pub fn switch_model(&mut self, id: ModelId) {
        let Some(session) = self.state.attached() else {
            self.draft.model = Some(id);
            return;
        };
        self.state.set_model(id.provider.clone(), id.model.clone());
        self.client
            .queue(Request::Post(Post::Switch { session, id }));
    }

    /// `/model` — the configured default; it takes effect on the next start.
    pub fn set_default_model(&mut self, id: ModelId) {
        self.client.queue(Request::Post(Post::DefaultModel { id }));
    }

    pub fn set_profile(&mut self, profile: String) {
        let Some(session) = self.state.attached() else {
            self.draft.profile = Some(profile);
            return;
        };
        self.client
            .queue(Request::Post(Post::Profile { session, profile }));
    }

    pub fn delete(&mut self, id: i64) {
        self.client
            .queue(Request::Post(Post::Delete { session: id }));
    }

    pub fn list_sessions(&mut self, under: Option<PathBuf>) {
        self.read(Call::Sessions { under });
    }

    pub fn usage(&mut self, lang: UsageLang) {
        if let Some(session) = self.state.attached() {
            self.read(Call::Usage { session, lang });
        }
    }

    pub fn todo(&mut self) {
        if let Some(session) = self.state.attached() {
            self.read(Call::Todo { session });
        }
    }

    pub fn pools(&mut self) {
        self.read(Call::Pools);
    }

    pub fn set_preload(&mut self, preload: usize) {
        self.window.set_budget(preload);
    }

    pub fn set_interrupted_text(&mut self, text: String) {
        self.window.set_placeholder_text(text);
    }

    /// Say something that never went near the wire; the caller names its lifetime.
    pub fn notice(&mut self, text: String, level: Level, life: Life) {
        self.state.notice(text, level, life);
    }

    /// The body of one block, if the window holds it: what a rewind re-offers.
    pub fn text_of(&self, block: i64) -> Option<&str> {
        let held = self.window.stored().iter().find(|b| b.key() == block)?;
        held.block.content.as_deref().or(held.block.text.as_deref())
    }

    pub fn view(&self) -> View<'_> {
        View {
            attached: self.state.attached(),
            busy: self.state.busy(),
            name: self.state.name(),
            model: self.state.model(),
            cwd: self.state.cwd(),
            cwd_temporary: self.state.cwd_temporary(),
            notices: self.state.notices(),
            refused: self.state.refused(),
            todo: self.state.todo(),
            rows: self.state.rows(),
            pools: self.state.pools(),
            usage: self.state.usage(),
            ctx_tokens: self.last_prompt_tokens(),
            ctx_limit: self.state.ctx_limit(),
            blocks: self.window.stored(),
            live: self.window.tail().blocks(),
            placeholder: self.window.placeholder(),
            tail: self.window.tail(),
            shapes: &self.shapes,
            range: &self.range,
            generation: self.window.generation(),
        }
    }
}

/// What a render side reads. Borrowed: drawing every frame must not copy the transcript.
pub struct View<'a> {
    /// The session this front end is on; `None` = a draft (no row yet).
    pub attached: Option<i64>,
    pub busy: bool,
    /// The name written by `/name`; `None` = unnamed.
    pub name: Option<&'a str>,
    /// This session's model; `None` = it follows the configured default.
    pub model: Option<&'a str>,
    /// Where the session works right now (the model's `cd` moves it).
    pub cwd: &'a str,
    /// The model moved the working directory: the session row says something else.
    pub cwd_temporary: bool,
    pub notices: &'a [Notice],
    /// The last write was refused because the session is mid-round.
    pub refused: bool,
    /// The task list, replayed by the server from the session's own tool calls.
    pub todo: &'a [TodoPhase],
    /// Prompt tokens of the newest reply: the context gauge's numerator.
    pub ctx_tokens: Option<u64>,
    /// The model's context window: the gauge's denominator, once a front end has asked.
    pub ctx_limit: Option<u64>,
    /// The picker's rows, as last asked for; `None` = not answered yet.
    pub rows: Option<&'a [SessionRow]>,
    pub pools: Option<&'a CandidatePools>,
    pub usage: Option<&'a UsagePanel>,
    /// Landed blocks, ascending by id.
    pub blocks: &'a VecDeque<WindowBlock>,
    /// The tail that has not landed, with the hash a finished piece carries.
    pub live: &'a [LiveBlock],
    /// The optimistic interrupt row, standing where the server's own will land.
    pub placeholder: Option<&'a NewBlock>,
    /// The streaming half sentence, and what a running tool has printed so far.
    pub tail: &'a LiveTail,
    /// Which blocks each kept round manages, newest first (the tree and a rewind read it).
    pub shapes: &'a Shapes,
    /// Raw blocks a tree asked for, independent of the content window.
    pub range: &'a RangeCache,
    /// A whole replacement moves it: a render side's finished cache is invalid on it.
    pub generation: u64,
}

fn next_budget(current: usize) -> usize {
    if current > 8 { 8 } else { (current / 2).max(4) }
}

impl<'a> View<'a> {
    /// Every block in display order: landed, then the tail, then the optimistic row.
    pub fn chain(&self) -> impl DoubleEndedIterator<Item = BlockRef<'a>> {
        let landed = self.blocks.iter().map(|b| BlockRef::Landed(&b.block));
        let live = self.live.iter().map(|l| BlockRef::Live(&l.block));
        let placeholder = self.placeholder.iter().copied().map(BlockRef::Live);
        landed.chain(live).chain(placeholder)
    }

    pub fn block_count(&self) -> usize {
        self.blocks.len() + self.live.len() + usize::from(self.placeholder.is_some())
    }

    /// The round a block belongs to: what a rewind names with the block. A block outside the
    /// window is answered from the kept structure.
    pub fn turn(&self, block: i64) -> Option<i64> {
        self.blocks
            .iter()
            .find(|b| b.block.id == block)
            .and_then(|b| b.turn)
            .or_else(|| self.shapes.turn_of(block))
    }

    /// The newest reply's prompt size: the numerator of the context gauge.
    pub fn last_prompt_tokens(&self) -> Option<u64> {
        let live = self.live.iter().rev().find_map(|l| {
            (l.block.kind == BlockKind::Assistant)
                .then_some(l.block.usage_prompt)
                .flatten()
        });
        live.or_else(|| {
            self.blocks.iter().rev().find_map(|b| {
                (b.block.kind == BlockKind::Assistant)
                    .then_some(b.block.usage_prompt)
                    .flatten()
            })
        })
    }
}
