//! The one inbound writer: arrivals become structure, content, flags, and outbox entries.

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::ipc::contract::ErrorCode;

use super::client::{Arrival, Call, Post, Push, Request, Subscribe};
use super::content::ContentCache;
use super::event::{
    BlocksReply, Change, ChildrenReply, ConfigValue, CreatedReply, CutReply, DefaultModelReply,
    Event, FailKind, InitReply, MemoReply, OpenedReply, SessionsReply,
};
use super::state::State;
use super::structure::{Structure, Turn};

/// Write one arrival into the caches, the flags, and the outbox.
pub fn apply(
    arrival: Arrival,
    state: &mut State,
    structure: &mut Structure,
    content: &mut ContentCache,
) {
    match arrival {
        Arrival::Push(push) => on_push(push, state, structure, content),
        Arrival::Answer { request, payload } => {
            on_answer(request, payload, state, structure, content)
        }
        Arrival::Refused {
            request,
            code,
            message,
        } => on_refused(request, code, &message, state),
        Arrival::Lost { request, message } => {
            clear_init(&request, state);
            forget_watch(&request, state);
            if mine(&request, state) {
                failed(state, request.fail_kind(), message);
            }
        }
    }
}

fn on_push(push: Push, state: &mut State, structure: &mut Structure, content: &mut ContentCache) {
    match push {
        Push::Piece { session, piece } => {
            if state.attached() != Some(session) {
                return;
            }
            state.set_busy(true);
            state.push(Event::Live(piece));
        }
        Push::Structure {
            session,
            turn,
            ranges,
        } => {
            if state.attached() != Some(session) {
                return;
            }
            let turn = Turn::new(turn, ranges);
            structure.set(turn.clone());
            state.set_busy(false);
            state.push(Event::Structure(turn));
        }
        Push::Notice {
            session,
            level,
            text,
        } => {
            if state.attached() != Some(session) {
                return;
            }
            state.push(Event::Notice { level, text });
        }
        Push::Change { session, change } => {
            if state.attached() != Some(session) {
                return;
            }
            match &change {
                Change::Deleted => {
                    state.detach();
                    structure.clear();
                    content.clear();
                }
                Change::Rewound { turn, block, cut } => {
                    structure.rewind(*turn, *block, *cut);
                    content.retain(|id| structure.turn_of(id).is_some());
                }
                Change::Meta(_) => {}
            }
            state.push(Event::Change(change));
        }
    }
}

fn on_answer(
    request: Request,
    payload: Value,
    state: &mut State,
    structure: &mut Structure,
    content: &mut ContentCache,
) {
    clear_init(&request, state);
    if !mine(&request, state) {
        return;
    }
    match request {
        Request::Call(call) => on_call(call, payload, state, structure, content),
        Request::Post(post) => on_post(post, payload, state, structure, content),
        Request::Subscribe(subscribe) => on_subscribe(subscribe, payload, state),
        Request::Bye => {}
    }
}

fn on_call(
    call: Call,
    payload: Value,
    state: &mut State,
    structure: &mut Structure,
    content: &mut ContentCache,
) {
    match call {
        Call::Init { .. } => match decode::<InitReply>(payload) {
            Ok(reply) => {
                let mut turns: Vec<Turn> = reply
                    .structure
                    .into_iter()
                    .map(|shape| Turn::new(shape.turn, shape.ranges))
                    .collect();
                turns.reverse();
                structure.replace(turns.clone());
                content.replace(reply.blocks.clone());
                if !state.busy() {
                    content.evict();
                }
                state.push(Event::Init {
                    structure: turns,
                    blocks: reply.blocks,
                    todo: reply.todo,
                });
            }
            Err(e) => failed(state, FailKind::Read, e),
        },
        Call::Blocks { .. } => match decode::<BlocksReply>(payload) {
            Ok(reply) => {
                content.merge(reply.blocks.clone());
                if !state.busy() {
                    content.evict();
                }
                state.push(Event::Blocks(reply.blocks));
            }
            Err(e) => failed(state, FailKind::Read, e),
        },
        Call::Meta { .. } => match decode(payload) {
            Ok(meta) => state.push(Event::Meta(meta)),
            Err(e) => failed(state, FailKind::Read, e),
        },
        Call::Usage { .. } => match decode(payload) {
            Ok(usage) => state.push(Event::Usage(usage)),
            Err(e) => failed(state, FailKind::Read, e),
        },
        Call::Todo { .. } => match decode(payload) {
            Ok(todo) => state.push(Event::Todo(todo)),
            Err(e) => failed(state, FailKind::Read, e),
        },
        Call::Memo { .. } => match decode::<MemoReply>(payload) {
            Ok(reply) => state.push(Event::Memo(reply.into_data())),
            Err(e) => failed(state, FailKind::Read, e),
        },
        Call::Children { .. } => match decode::<ChildrenReply>(payload) {
            Ok(reply) => state.push(Event::Children(reply.rows)),
            Err(e) => failed(state, FailKind::Read, e),
        },
        Call::Config { key } => {
            state.push(Event::Config {
                key,
                value: ConfigValue::from_json(payload),
            });
        }
        Call::Pools => match decode(payload) {
            Ok(pools) => state.push(Event::Pools(pools)),
            Err(e) => failed(state, FailKind::Read, e),
        },
        Call::Sessions { .. } => match decode::<SessionsReply>(payload) {
            Ok(reply) => state.push(Event::Sessions(reply.rows)),
            Err(e) => failed(state, FailKind::Read, e),
        },
        Call::Budget => {
            if let Some(budget) = payload.as_i64() {
                content.set_budget(budget.max(1) as usize);
            }
        }
    }
}

fn on_post(
    post: Post,
    payload: Value,
    state: &mut State,
    structure: &mut Structure,
    content: &mut ContentCache,
) {
    match post {
        Post::Init { .. } => match decode::<CreatedReply>(payload) {
            Ok(reply) => {
                let session = reply.session;
                state.attach(session);
                structure.clear();
                content.clear();
                state.set_new_born(true);
                state.push(Event::Created(session));
            }
            Err(e) => failed(state, FailKind::Change, e),
        },
        Post::Rewind { turn, block, .. } => match decode::<CutReply>(payload) {
            Ok(reply) => {
                structure.rewind(turn, block, reply.cut);
                content.retain(|id| structure.turn_of(id).is_some());
                state.set_refused(false);
                state.push(Event::Cut(reply.cut));
            }
            Err(e) => failed(state, FailKind::Change, e),
        },
        Post::Delete { session } => {
            if state.attached() == Some(session) {
                state.detach();
                structure.clear();
                content.clear();
            }
            state.set_refused(false);
            state.push(Event::Done);
        }
        Post::Name { .. } | Post::Cwd { .. } | Post::Switch { .. } | Post::Interrupt { .. } => {
            state.set_refused(false);
            state.push(Event::Done);
        }
        Post::DefaultModel { .. } => match decode::<DefaultModelReply>(payload) {
            Ok(reply) => {
                state.set_refused(false);
                state.push(Event::DefaultModel(reply.path));
            }
            Err(e) => failed(state, FailKind::Change, e),
        },
    }
}

fn on_subscribe(subscribe: Subscribe, payload: Value, state: &mut State) {
    match subscribe {
        Subscribe::Watch { .. } => {}
        Subscribe::Submit { .. } | Subscribe::Compact { .. } => {
            match decode::<OpenedReply>(payload) {
                Ok(reply) => {
                    state.set_busy(true);
                    state.set_refused(false);
                    state.push(Event::Opened { turn: reply.turn });
                }
                Err(e) => failed(state, FailKind::Round, e),
            }
        }
    }
}

fn on_refused(request: Request, code: ErrorCode, message: &str, state: &mut State) {
    clear_init(&request, state);
    forget_watch(&request, state);
    if !mine(&request, state) {
        return;
    }
    if code == ErrorCode::Busy {
        state.set_busy(true);
        state.set_refused(true);
        state.push(Event::Busy);
    } else {
        failed(state, request.fail_kind(), message.to_string());
    }
}

fn mine(request: &Request, state: &State) -> bool {
    request
        .session()
        .map_or(true, |session| state.attached() == Some(session))
}

fn forget_watch(request: &Request, state: &mut State) {
    if matches!(request, Request::Subscribe(Subscribe::Watch { .. })) {
        state.set_watching(None);
    }
}

fn clear_init(request: &Request, state: &mut State) {
    if matches!(request, Request::Call(Call::Init { .. })) {
        state.set_init(None);
    }
}

fn failed(state: &mut State, kind: FailKind, message: String) {
    state.push(Event::Failed { kind, message });
}

fn decode<T: DeserializeOwned>(payload: Value) -> Result<T, String> {
    serde_json::from_value(payload).map_err(|e| e.to_string())
}
