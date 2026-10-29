//! Child sessions: the sessions a `task` call spawned, with their current state.

use anyhow::Result;

use crate::server::api::call::ctx::Ctx;
use crate::server::api::call::server::sessions::row;
use crate::server::api::call::session::block_ids;

use crate::ipc::contract::{ChildRow, ChildState};

pub(crate) fn list(ctx: &Ctx, session: i64) -> Result<Vec<ChildRow>> {
    let anchors = block_ids(ctx.reader, session)?;
    let mut stmt = ctx
        .reader
        .prepare("SELECT subsession_id, anchor FROM subsessions")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;

    let mut out = Vec::new();
    for found in rows {
        let (child, anchor) = found?;
        if !anchors.contains(&anchor) {
            continue;
        }
        let session = crate::server::api::call::server::sessions::row(ctx.reader, child)?;
        let intent = intent(ctx, anchor)?;
        let state = if ctx.running.flag(child).is_some() {
            ChildState::Running
        } else {
            let turns: i64 = ctx.reader.query_row(
                "SELECT COUNT(*) FROM session_turns WHERE session_id = ?1",
                [child],
                |r| r.get(0),
            )?;
            if turns > 0 {
                ChildState::Ended
            } else {
                ChildState::Queued
            }
        };
        out.push(ChildRow {
            anchor,
            session,
            intent,
            state,
        });
    }
    out.sort_by_key(|c| c.anchor);
    Ok(out)
}

fn intent(ctx: &Ctx, anchor: i64) -> Result<Option<String>> {
    let call_id: Option<String> = ctx
        .reader
        .query_row(
            "SELECT call_id FROM blocks WHERE block_id = ?1",
            [anchor],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    let Some(call_id) = call_id else {
        return Ok(None);
    };
    Ok(ctx
        .reader
        .query_row(
            "SELECT intent FROM blocks
              WHERE call_id = ?1 AND type = 'tool_request' AND name = 'task'
              ORDER BY block_id LIMIT 1",
            [call_id],
            |r| r.get(0),
        )
        .ok()
        .flatten())
}
