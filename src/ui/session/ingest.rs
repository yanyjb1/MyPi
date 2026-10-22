//! The backend's arrivals → the session state and the raw cache, the one writer from outside.
//! Design: `README.md`.

use crate::server::api::{Call, Opened, PostDone, Reply, StreamItem};
use crate::server::storage::BlockKind;

use super::client::{Arrival, Kind, Round};
use super::range::RangeCache;
use super::shape::Shapes;
use super::state::{Level, Life, State};
use super::window::Window;

/// What an arrival asks the session to do next. The session turns it into a request.
pub enum Follow {
    Nothing,
    /// A row was created just now: enter it, and send the message that was waiting for it.
    Created(i64),
    /// An existing session was entered.
    Watch(i64),
    /// The task list may have changed.
    Todo(i64),
    /// A round just ended: its structure is worth asking for.
    Shape {
        session: i64,
        turn: i64,
    },
}

pub fn apply(
    arrival: Arrival,
    state: &mut State,
    window: &mut Window,
    shapes: &mut Shapes,
    range: &mut RangeCache,
) -> Follow {
    match arrival {
        Arrival::Stream(item) => on_stream(item, state, window, shapes),
        Arrival::Answer { call, reply } => on_answer(call, reply, state, window, shapes, range),
        Arrival::Done(done) => on_done(done, state),
        Arrival::Opened {
            session,
            round,
            opened,
        } => on_opened(session, round, opened, state, window),
        Arrival::WatchEnded => {
            state.set_busy(false);
            state.notice("这条会话的推送断了".to_string(), Level::Error, Life::Long);
            Follow::Nothing
        }
        Arrival::Failed {
            kind,
            call: _,
            message,
        } => {
            if matches!(kind, Kind::Round | Kind::Watch) {
                state.set_busy(false);
            }
            state.notice(message, Level::Error, Life::Long);
            Follow::Nothing
        }
    }
}

fn on_stream(item: StreamItem, state: &mut State, window: &mut Window, shapes: &Shapes) -> Follow {
    match item {
        StreamItem::Live { blocks, done } => window.push_live(blocks, done),
        StreamItem::Blocks(blocks) => {
            let asked_todo = blocks
                .iter()
                .any(|b| b.kind == BlockKind::ToolResult && b.name.as_deref() == Some("todo"));
            window.push_blocks(blocks, state.busy());
            // A batch can land late: the kept structure says which round it belongs to.
            restamp(window, shapes, state.busy());
            state.settle();
            if asked_todo && let Some(id) = state.attached() {
                return Follow::Todo(id);
            }
        }
        // Display only: the result block carries the durable text.
        StreamItem::ToolOutput { call_id, text } => window.set_tool_output(call_id, text),
        StreamItem::Ended { turn, .. } => {
            state.set_busy(false);
            if let Some(turn) = turn {
                window.end_round(turn);
                if let Some(session) = state.attached() {
                    return Follow::Shape { session, turn };
                }
            }
        }
    }
    Follow::Nothing
}

/// Write the kept rounds onto the window again, matching by id and content hash.
fn restamp(window: &mut Window, shapes: &Shapes, busy: bool) {
    for shape in shapes.iter() {
        window.claim(shape.turn, &shape.blocks, busy);
    }
}

fn on_answer(
    call: Call,
    reply: Reply,
    state: &mut State,
    window: &mut Window,
    shapes: &mut Shapes,
    range: &mut RangeCache,
) -> Follow {
    match reply {
        Reply::Init {
            turns,
            blocks,
            more,
        } => {
            if call_session(&call) == state.attached() {
                shapes.replace(turns);
                window.replace(blocks, more);
                restamp(window, shapes, state.busy());
            }
        }
        Reply::Round {
            turn: _,
            blocks,
            older,
            newer,
        } => {
            if call_session(&call) == state.attached() {
                window.replace_round(blocks, older, newer);
            }
        }
        Reply::Range(blocks) => {
            if call_session(&call) == state.attached()
                && let Call::Range { start, end, .. } = call
            {
                range.replace(start, end, blocks);
            }
        }
        Reply::Blocks { blocks, more } => {
            if call_session(&call) == state.attached() {
                match call {
                    Call::Older { turn, .. } => window.prepend(blocks, more, turn),
                    Call::Newer { turn, .. } => window.append(blocks, more, turn),
                    _ => window.replace(blocks, more),
                }
            }
        }
        Reply::Turn { turn, blocks } => {
            if call_session(&call) == state.attached() {
                shapes.set(turn, blocks);
                restamp(window, shapes, state.busy());
            }
        }
        Reply::Meta(meta) => {
            if call_session(&call) == state.attached() {
                state.set_meta(meta);
            }
        }
        Reply::Usage { lang, groups } => {
            if call_session(&call) == state.attached() {
                state.set_usage(lang, groups);
            }
        }
        Reply::SessionRows(rows) => state.set_rows(rows),
        Reply::Todo(list) => {
            if call_session(&call) == state.attached() {
                state.set_todo(list);
            }
        }
        Reply::Pools(pools) => state.set_pools(pools),
    }
    Follow::Nothing
}

fn on_done(done: PostDone, state: &mut State) -> Follow {
    match done {
        PostDone::Created(id) => {
            state.set_refused(false);
            state.set_new_born(true);
            Follow::Created(id)
        }
        PostDone::Done => {
            state.set_refused(false);
            Follow::Nothing
        }
        PostDone::Busy => {
            state.set_refused(true);
            Follow::Nothing
        }
    }
}

fn on_opened(
    session: i64,
    round: Round,
    opened: Opened,
    state: &mut State,
    window: &mut Window,
) -> Follow {
    if state.attached() != Some(session) {
        return Follow::Nothing;
    }
    match opened {
        Opened::Started { turn } => {
            state.set_busy(true);
            if let Some(turn) = turn {
                window.set_turn(turn);
            }
        }
        // Nothing started: already mid-round, or nothing to compact.
        Opened::Busy => {
            state.set_busy(true);
            let what = match round {
                Round::Submit => "上一轮还没跑完，这条要等它结束",
                Round::Compact => "没有可压缩的东西（或者这一轮还没跑完）",
            };
            state.notice(what.to_string(), Level::Info, Life::Short);
        }
    }
    Follow::Nothing
}

fn call_session(call: &Call) -> Option<i64> {
    match call {
        Call::Usage { session, .. }
        | Call::Init { session, .. }
        | Call::Round { session, .. }
        | Call::Range { session, .. }
        | Call::Older { session, .. }
        | Call::Newer { session, .. }
        | Call::Todo { session }
        | Call::Turn { session, .. }
        | Call::Meta { session } => Some(*session),
        Call::Sessions { .. } | Call::Pools => None,
    }
}
