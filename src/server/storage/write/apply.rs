//! One submission is one transaction: the rows run in order, either all or none.

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::super::shape::{Landed, Ref, Relay, Row, Submit};
use super::{aliases, artifacts, blocks, memos, sessions, shared, subsessions, turns};

/// Carry one submission out. Ids the rows cannot know come back here; the relay pieces are
/// returned for the writer's port.
pub(crate) fn apply(conn: &mut Connection, submit: Submit) -> Result<(Landed, Vec<Relay>)> {
    let tx = conn.transaction()?;
    let mut ids = vec![0i64; submit.rows.len()];
    let mut pieces: Vec<Relay> = Vec::new();
    for (at, row) in submit.rows.iter().enumerate() {
        ids[at] = one(&tx, row, &ids, &mut pieces)?;
    }
    tx.commit()?;
    Ok((Landed { ids }, pieces))
}

/// Carry one row out, returning the id it produced (`0` when it produced none).
fn one(
    tx: &rusqlite::Transaction<'_>,
    row: &Row,
    ids: &[i64],
    pieces: &mut Vec<Relay>,
) -> Result<i64> {
    Ok(match row {
        Row::Session {
            started_at,
            cwd,
            model,
            name,
        } => sessions::insert(tx, started_at, cwd, model.as_ref(), name.as_deref())?,
        Row::Rename { session, name } => {
            sessions::rename(tx, *session, name.as_deref())?;
            0
        }
        Row::Switch { session, model } => {
            sessions::switch(tx, *session, model.as_ref())?;
            0
        }
        Row::MoveCwd { session, cwd } => {
            sessions::move_cwd(tx, *session, cwd)?;
            0
        }
        Row::TodoOn { session, on } => {
            sessions::todo_on(tx, *session, *on)?;
            0
        }
        Row::Turn {
            request,
            stop_reason,
            usage,
        } => {
            let request = match request {
                Some(r) => Some(id_of(ids, r)?),
                None => None,
            };
            turns::insert(tx, request, stop_reason.as_deref(), *usage)?
        }
        Row::Settle {
            turn,
            stop_reason,
            usage,
        } => {
            let turn = id_of(ids, turn)?;
            turns::settle(tx, turn, stop_reason.as_deref(), *usage)?;
            if let Some(session) = shared::session_of_turn(tx, turn)? {
                pieces.push(Relay::Ended {
                    session,
                    turn,
                    usage: *usage,
                    stop_reason: stop_reason.clone(),
                });
            }
            0
        }
        Row::Request { meta } => shared::put_request_meta(tx, meta)?,
        Row::Block { turn, block } => {
            let turn = id_of(ids, turn)?;
            let landed = blocks::insert(tx, turn, block)?;
            if let Some(session) = shared::session_of_turn(tx, turn)? {
                pieces.push(Relay::Blocks {
                    session,
                    turn,
                    blocks: vec![landed],
                });
            }
            0
        }
        Row::Artifact {
            block,
            name,
            content,
        } => {
            artifacts::insert(tx, id_of(ids, block)?, name, content)?;
            0
        }
        Row::Memo {
            anchor,
            name,
            content,
            ts,
        } => {
            let anchor = match anchor {
                Some(a) => Some(id_of(ids, a)?),
                None => None,
            };
            memos::insert(tx, anchor, name, content, ts)?;
            0
        }
        Row::Alias {
            anchor,
            name,
            command,
            ts,
        } => {
            aliases::insert(tx, id_of(ids, anchor)?, name, command, ts)?;
            0
        }
        Row::Subsession {
            subsession_id,
            anchor,
        } => {
            subsessions::insert(tx, *subsession_id, id_of(ids, anchor)?)?;
            0
        }
        Row::Cwd { session, turn, cwd } => {
            sessions::cwd_row(tx, *session, id_of(ids, turn)?, cwd)?;
            0
        }
        Row::List {
            session,
            turn,
            is_compaction,
        } => {
            shared::append_list_row(tx, *session, id_of(ids, turn)?, *is_compaction)?;
            0
        }
    })
}

/// The id a reference names: a number the caller knew, or an earlier row's own id.
fn id_of(ids: &[i64], r: &Ref) -> Result<i64> {
    match r {
        Ref::Known(id) => Ok(*id),
        Ref::Batch(at) => ids
            .get(*at)
            .copied()
            .filter(|id| *id > 0)
            .with_context(|| format!("row {at} produced no id")),
    }
}
