//! The one inbound writer: upstream objects become structure, content, flags, and outbox entries.

use std::ops::RangeInclusive;

use crate::ipc::contract::{
    Call, ContentEvent, Event, Opened, PostDone, Reply, SessionEvent, SessionReply, ToolAsk,
    TurnShape,
};

use super::client::{Arrival, Kind};
use super::content::ContentCache;
use super::state::State;
use super::structure::{Structure, Turn};

/// Write one arrival into the caches, the flags, and the outbox.
pub fn apply(
    arrival: Arrival,
    state: &mut State,
    structure: &mut Structure,
    content: &mut ContentCache,
) {
    let mine = belongs(&arrival, state.attached());
    if mine {
        match &arrival {
            Arrival::Stream(event) => on_stream(event, state, structure, content),
            Arrival::Answer { call, reply } => on_answer(call, reply, structure, content),
            Arrival::Done(done) => on_done(done, state),
            Arrival::Opened {
                session, opened, ..
            } => on_opened(*session, opened, state),
            Arrival::WatchEnded => state.set_busy(false),
            Arrival::Failed { kind, .. } => {
                if matches!(kind, Kind::Round | Kind::Watch) {
                    state.set_busy(false);
                }
            }
        }
    }
    if mine {
        state.push(arrival);
    }
}

/// Does this arrival belong to the attached session? A failure or a watch end is always ours.
fn belongs(arrival: &Arrival, attached: Option<i64>) -> bool {
    match arrival {
        Arrival::Stream(event) => attached == Some(event.session()),
        Arrival::Answer { call, .. } => match call_session(call) {
            Some(session) => attached == Some(session),
            None => true,
        },
        Arrival::Opened { session, .. } => Some(*session) == attached,
        Arrival::WatchEnded | Arrival::Done(_) | Arrival::Failed { .. } => true,
    }
}

fn on_stream(
    event: &Event,
    state: &mut State,
    structure: &mut Structure,
    content: &mut ContentCache,
) {
    match event {
        Event::Content(event) => match event {
            ContentEvent::Started { .. } => state.set_busy(true),
            ContentEvent::Live { blocks, done, .. } => {
                for block in blocks {
                    content.push_live(block.clone(), *done);
                }
            }
            ContentEvent::Blocks { turn, blocks, .. } => {
                structure.add_blocks(*turn, blocks);
                content.merge_landed(blocks.clone());
            }
            ContentEvent::ToolOutput { call_id, text, .. } => {
                content.set_tool_output(call_id.clone(), text.clone());
            }
            ContentEvent::Ended { .. } => {
                state.set_busy(false);
                content.clear_tool_output();
            }
        },
        Event::Session(event) => match event {
            SessionEvent::Meta { .. } => {}
            SessionEvent::Rewound { .. } => state.set_busy(false),
            SessionEvent::Deleted { .. } => {
                state.detach();
                structure.clear();
                content.clear();
            }
        },
        Event::Notice(_) => {}
    }
}

fn on_answer(call: &Call, reply: &Reply, structure: &mut Structure, content: &mut ContentCache) {
    match reply {
        Reply::Session(SessionReply::Init(init)) => {
            let turns = init.structure.iter().cloned().map(turn_of).collect();
            structure.replace(Structure::new(turns));
            content.replace_landed(init.blocks.clone());
        }
        Reply::Session(SessionReply::Blocks(blocks)) => content.merge_landed(blocks.clone()),
        _ => {}
    }
}

fn on_done(done: &PostDone, state: &mut State) {
    match done {
        PostDone::Created(_) => {
            state.set_refused(false);
            state.set_new_born(true);
        }
        PostDone::Done => state.set_refused(false),
        PostDone::Busy => state.set_refused(true),
    }
}

fn on_opened(session: i64, opened: &Opened, state: &mut State) {
    if state.attached() != Some(session) {
        return;
    }
    match opened {
        Opened::Started { .. } | Opened::Busy => state.set_busy(true),
    }
}

fn call_session(call: &Call) -> Option<i64> {
    match call {
        Call::Session { session, .. } => Some(*session),
        Call::Tool(ToolAsk::Todo { session }) | Call::Tool(ToolAsk::Children { session }) => {
            Some(*session)
        }
        Call::Tool(ToolAsk::Memo { session, .. }) => *session,
        Call::Server(_) => None,
    }
}

/// One round's wire shape: its block ids, folded into intervals in id order.
fn turn_of(shape: TurnShape) -> Turn {
    let mut ids: Vec<i64> = shape.blocks.iter().map(|block| block.id).collect();
    ids.sort_unstable();
    let mut blocks: Vec<RangeInclusive<i64>> = Vec::new();
    for id in ids {
        match blocks.last_mut() {
            Some(last) if *last.end() + 1 == id => *last = *last.start()..=id,
            _ => blocks.push(id..=id),
        }
    }
    Turn {
        id: shape.turn,
        blocks,
    }
}
