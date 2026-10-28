//! Closed block ranges: request order, ascending inside a range, overlaps kept.

use anyhow::{Result, bail};
use std::collections::HashMap;

use crate::ipc::contract::Block;
use crate::server::api::call::ctx::Ctx;
use crate::server::api::convert;

use super::{blocks_of, list_ids, turn};

pub(crate) fn answer(ctx: &Ctx, session: i64, ranges: Vec<[i64; 2]>) -> Result<Vec<Block>> {
    let mut by_id: HashMap<i64, Block> = HashMap::new();
    for turn_id in list_ids(ctx.reader, session, 0)? {
        if let Some(row) = turn(ctx.reader, turn_id)? {
            for block in blocks_of(ctx.reader, &row)? {
                by_id.insert(block.id, convert::block(block));
            }
        }
    }

    let mut out = Vec::new();
    for [lo, hi] in ranges {
        if lo > hi {
            bail!("a block range is reversed: [{lo}, {hi}]");
        }
        for id in lo..=hi {
            let Some(block) = by_id.get(&id) else {
                bail!("block {id} is not in session {session}");
            };
            out.push(block.clone());
        }
    }
    Ok(out)
}
