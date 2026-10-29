//! The write inlet: what lands when, and what is relayed at once. Three things live here in
//! memory: the batch counter, the tail that has not landed, and the port pieces are relayed to.

use std::sync::mpsc::Sender;

use anyhow::{Context, Result};

use super::super::shape::{
    BlockKind, Effect, EffectKind, Landed, NewBlock, Relay, RoundUsage, SessionChange, SessionRef,
    Submit,
};
use super::super::write::shared::open_turn;
use super::Store;

/// The in-memory working state of the write path.
pub(crate) struct Inlet {
    /// How many tool rounds land one batch (the config's number, handed in).
    batch: usize,
    /// The round the unlanded blocks belong to, those blocks, and their effects.
    pending: Option<(i64, Vec<NewBlock>, Vec<Effect>)>,
    /// The blocks this round is still writing (`done: false` pieces accumulate here).
    open: Vec<NewBlock>,
    /// Tool rounds handed in since the last landing.
    rounds: usize,
    /// Where new pieces are relayed (the api's port; `None` = nobody listens).
    pub(crate) relay: Option<Sender<Relay>>,
}

impl Inlet {
    /// A fresh inlet. `batch` comes from the config and is never zero.
    pub(crate) fn new(batch: usize) -> Self {
        Self {
            batch: batch.max(1),
            pending: None,
            open: Vec::new(),
            rounds: 0,
            relay: None,
        }
    }

    /// Mark a pending or open tool result, if this call is still in memory.
    pub(crate) fn mark_todo(&mut self, call_id: &str, value: &str) -> bool {
        let mut hit = false;
        if let Some((_, pieces, _)) = &mut self.pending {
            for block in pieces.iter_mut() {
                if is_todo_block(block, call_id) {
                    block.memo = Some(value.to_string());
                    hit = true;
                }
            }
        }
        for block in &mut self.open {
            if is_todo_block(block, call_id) {
                block.memo = Some(value.to_string());
                hit = true;
            }
        }
        hit
    }
}

impl Store {
    /// Carry one submission out: it lands (now or later), and what a surface
    /// has to see is relayed at once.
    pub(crate) fn land(&mut self, submit: Submit) -> Result<Landed> {
        match submit {
            Submit::NewSession {
                started_at,
                cwd,
                model,
                name,
                profile,
            } => {
                let pair = model.as_ref().map(|(p, m)| (p.as_str(), m.as_str()));
                let id = self.open_session(
                    &started_at,
                    &cwd,
                    pair,
                    name.as_deref(),
                    profile.as_deref(),
                )?;
                Ok(Landed {
                    session: Some(id),
                    ..Landed::default()
                })
            }
            Submit::UserMessage {
                session,
                prefix,
                text,
                suffix,
                ts,
            } => self.push_user_message(session, prefix, text, suffix, ts),
            Submit::Wake { session, text, ts } => self.push_wake(session, text, ts),
            Submit::Blocks {
                session,
                blocks,
                done,
                effects,
            } => self.push_blocks(session, blocks, done, effects),
            Submit::RoundEnd {
                session,
                stop_reason,
                usage,
            } => self.push_round_end(session, stop_reason, usage),
            Submit::Request { session, meta } => {
                let turn = open_turn(&self.conn, session)?.context(
                    "this session has no open round — the user's message must land first",
                )?;
                self.attach_request(turn, &meta)?;
                Ok(Landed::default())
            }
            Submit::Session { session, change } => {
                self.apply_change(session, change)?;
                Ok(Landed::default())
            }
            Submit::Compaction {
                session,
                summary,
                request,
                kept,
                profile,
            } => {
                let turn =
                    self.append_compaction(session, &summary, request.as_deref(), &kept, &profile)?;
                // A compaction is a round like any other: it ends when it lands.
                self.relay_out(Relay::Ended {
                    session,
                    turn,
                    usage: None,
                    stop_reason: Some("compaction".to_string()),
                });
                Ok(Landed {
                    turn: Some(turn),
                    ..Landed::default()
                })
            }
        }
    }

    /// The user's message: create the session if this is a fresh one, open the
    /// round and land it at once.
    fn push_user_message(
        &mut self,
        session: SessionRef,
        prefix: Option<String>,
        text: String,
        suffix: Option<String>,
        ts: Option<String>,
    ) -> Result<Landed> {
        let (session, created) = match session {
            SessionRef::Id(id) => (id, None),
            SessionRef::New { started_at, cwd } => {
                let id = self.create_session(&started_at, &cwd)?;
                (id, Some(id))
            }
        };
        let block = NewBlock {
            kind: BlockKind::User,
            content: Some(text),
            prefix,
            suffix,
            ts,
            ..NewBlock::default()
        };
        let (turn, landed) = self.open_turn(session, std::slice::from_ref(&block), None)?;
        self.inlet.pending = None;
        self.inlet.open.clear();
        self.inlet.rounds = 0;
        self.relay_out(Relay::Blocks {
            session,
            turn,
            blocks: landed.clone(),
        });
        Ok(Landed {
            session: created,
            turn: Some(turn),
            blocks: landed,
        })
    }

    fn push_wake(&mut self, session: i64, text: String, ts: Option<String>) -> Result<Landed> {
        let block = NewBlock {
            kind: BlockKind::Wake,
            content: Some(text),
            prefix: Some("[system wake: ".to_string()),
            suffix: Some("]".to_string()),
            ts,
            ..NewBlock::default()
        };
        let (turn, landed) = self.open_turn(session, std::slice::from_ref(&block), None)?;
        self.inlet.pending = None;
        self.inlet.open.clear();
        self.inlet.rounds = 0;
        self.relay_out(Relay::Blocks {
            session,
            turn,
            blocks: landed.clone(),
        });
        Ok(Landed {
            turn: Some(turn),
            blocks: landed,
            ..Landed::default()
        })
    }

    /// A reply's blocks (or a tool round's): relayed now, held for the landing. `done` = finished;
    /// an open piece extends the open block of the same kind and never lands.
    fn push_blocks(
        &mut self,
        session: i64,
        blocks: Vec<NewBlock>,
        done: bool,
        effects: Vec<Effect>,
    ) -> Result<Landed> {
        anyhow::ensure!(
            done || effects.is_empty(),
            "an open block cannot carry effects"
        );
        let turn = open_turn(&self.conn, session)?
            .context("this session has no open round — the user's message must land first")?;
        // What goes out is the visible stretch; the citation is added when the block lands.
        self.relay_out(Relay::Tail {
            session,
            turn,
            blocks: blocks
                .iter()
                .cloned()
                .map(|mut b| {
                    b.spill = None;
                    b
                })
                .collect(),
            done,
        });
        // A different round's tail must never be dropped on the floor: land it first.
        if self
            .inlet
            .pending
            .as_ref()
            .is_some_and(|(held, _, _)| *held != turn)
        {
            self.land_pending(session)?;
        }

        if !done {
            for block in blocks {
                match self
                    .inlet
                    .open
                    .iter_mut()
                    .find(|open| same_block(open, &block))
                {
                    Some(open) => grow(open, &block),
                    None => self.inlet.open.push(block),
                }
            }
            return Ok(Landed::default());
        }

        let mut finished = blocks;
        for block in &mut finished {
            if block.memo.is_none()
                && let Some(open) = self.inlet.open.iter().find(|open| same_block(open, block))
            {
                block.memo = open.memo.clone();
            }
        }
        for block in &finished {
            self.inlet.open.retain(|open| !same_block(open, block));
        }
        let tool_round = finished.iter().any(|b| b.kind == BlockKind::ToolResult);
        let anchor = effects
            .iter()
            .any(|e| matches!(&e.kind, EffectKind::Subsession { .. }));
        match &mut self.inlet.pending {
            Some((held, pieces, pending_effects)) if *held == turn => {
                pieces.append(&mut finished);
                pending_effects.extend(effects);
            }
            slot => *slot = Some((turn, finished, effects)),
        }
        if tool_round {
            self.inlet.rounds += 1;
        }
        if anchor || self.inlet.rounds >= self.inlet.batch {
            return self.land_pending(session);
        }
        Ok(Landed::default())
    }

    /// The round is over: land what is held and settle it.
    fn push_round_end(
        &mut self,
        session: i64,
        stop_reason: Option<String>,
        usage: Option<RoundUsage>,
    ) -> Result<Landed> {
        let turn = open_turn(&self.conn, session)?;
        let mut out = match turn {
            Some(_) => self.land_pending(session)?,
            None => Landed::default(),
        };
        self.inlet.rounds = 0;
        if let Some(turn) = turn {
            self.finish_turn(turn, stop_reason.as_deref(), usage)?;
            out.turn = Some(turn);
            self.relay_out(Relay::Ended {
                session,
                turn,
                usage,
                stop_reason,
            });
        }
        Ok(out)
    }

    /// Land everything held for the session's open round: a block still open lands as it stands.
    fn land_pending(&mut self, session: i64) -> Result<Landed> {
        let open = std::mem::take(&mut self.inlet.open);
        if !open.is_empty()
            && let Some(turn) = open_turn(&self.conn, session)?
        {
            if self
                .inlet
                .pending
                .as_ref()
                .is_some_and(|(held, _, _)| *held != turn)
            {
                self.land_pending(session)?;
            }
            match &mut self.inlet.pending {
                Some((held, pieces, _)) if *held == turn => pieces.extend(open),
                slot => *slot = Some((turn, open, Vec::new())),
            }
        }
        let Some((turn, blocks, effects)) = self.inlet.pending.take() else {
            return Ok(Landed::default());
        };
        self.inlet.rounds = 0;
        let landed = self.append_blocks(session, turn, &blocks, &effects)?;
        self.relay_out(Relay::Blocks {
            session,
            turn,
            blocks: landed.clone(),
        });
        Ok(Landed {
            session: None,
            turn: Some(turn),
            blocks: landed,
        })
    }

    /// One change to a session row.
    fn apply_change(&mut self, session: i64, change: SessionChange) -> Result<()> {
        match change {
            SessionChange::Name(name) => self.set_session_name(session, name.as_deref()),
            SessionChange::Model(model) => {
                let pair = model.as_ref().map(|(p, m)| (p.as_str(), m.as_str()));
                self.set_session_model(session, pair)
            }
            SessionChange::Cwd { turn, path } => self.set_session_cwd(session, turn, &path),
            SessionChange::Profile(name) => self.set_session_profile(session, &name),
            SessionChange::Todo { on, call_id } => self.set_todo_on(session, on, &call_id),
        }
    }

    /// Hand one piece to the relay port, if anybody listens.
    pub(crate) fn relay_out(&mut self, piece: Relay) {
        let dead = match self.inlet.relay.as_ref() {
            Some(port) => port.send(piece).is_err(),
            None => false,
        };
        if dead {
            // The api's receiver is gone: stop relaying to it.
            self.inlet.relay = None;
        }
    }
}

/// Is this piece part of that block? Two blocks are one block when they are the
/// same kind and, for a tool call, carry the same `call_id`.
fn same_block(a: &NewBlock, b: &NewBlock) -> bool {
    a.kind == b.kind && a.call_id == b.call_id
}

/// Is this the todo tool result for `call_id`?
fn is_todo_block(b: &NewBlock, call_id: &str) -> bool {
    b.kind == BlockKind::ToolResult && b.call_id.as_deref() == Some(call_id)
}

/// Take one more piece of a block that is still being written: the text grows,
/// everything else is already there from the first piece.
fn grow(open: &mut NewBlock, piece: &NewBlock) {
    match (&mut open.content, &piece.content) {
        (Some(text), Some(more)) => text.push_str(more),
        (None, Some(more)) => open.content = Some(more.clone()),
        _ => {}
    }
}
