//! The first frame: the whole structure, plus the newest blocks.

use anyhow::Result;

use crate::ipc::contract::Block;
use crate::ipc::contract::call;
use crate::server::api::call::ctx::Ctx;
use crate::server::api::call::tool::todo;
use crate::server::api::convert;

use super::{blocks_of, borrowed, list_ids, read_start, turn};

pub(crate) fn answer(ctx: &Ctx, session: i64, limit: usize) -> Result<call::init::Reply> {
    let from = read_start(ctx.reader, session)?.unwrap_or(0);
    let ids = list_ids(ctx.reader, session, from)?;

    let mut structure = Vec::with_capacity(ids.len());
    for turn_id in ids.iter().rev() {
        let Some(row) = turn(ctx.reader, *turn_id)? else {
            continue;
        };
        let mut blocks: Vec<call::init::TurnBlock> = blocks_of(ctx.reader, &row)?
            .into_iter()
            .map(|b| call::init::TurnBlock {
                id: b.id,
                hash: b.content_key().digest(),
            })
            .collect();
        blocks.reverse();
        structure.push(call::init::TurnShape {
            turn: *turn_id,
            blocks,
        });
    }

    let mut blocks = Vec::new();
    'outer: for turn_id in ids.iter().rev() {
        let Some(row) = turn(ctx.reader, *turn_id)? else {
            continue;
        };
        if borrowed(&row) {
            continue;
        }
        let mut list = blocks_of(ctx.reader, &row)?;
        list.reverse();
        for block in list {
            if blocks.len() == limit {
                break 'outer;
            }
            blocks.push(convert::block(block));
        }
    }
    blocks.reverse();

    let todo = match todo::enabled(ctx.reader, session)? {
        (true, cwd) => {
            let list = todo::replay(ctx.reader, session, cwd)?;
            if list.is_empty() {
                None
            } else {
                Some(list.into_iter().map(convert::todo_phase).collect())
            }
        }
        (false, _) => None,
    };

    Ok(call::init::Reply {
        structure,
        blocks,
        todo,
    })
}
