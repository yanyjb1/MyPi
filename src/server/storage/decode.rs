//! Decoding: a database row becomes a [`shape`](super::shape) value. It is internal to storage: the
//! reader uses it, and no caller names a column.

use anyhow::{Context, Result};

use super::shape::{Block, BlockKind, RequestMeta, RoundUsage, SessionMeta, TurnRow};

/// Decode a run of interval ids into `Vec<[i64; 2]>`. An unreadable value is
/// a broken row: error, never silently empty.
pub(crate) fn parse_blocks(raw: &str) -> Result<Vec<[i64; 2]>> {
    serde_json::from_str(raw)
        .with_context(|| format!("a round's block index is unreadable: {raw:.80}"))
}

/// Encode interval ids as JSON.
pub(crate) fn encode_blocks(blocks: &[[i64; 2]]) -> String {
    serde_json::to_string(blocks).expect("interval pairs always serialize")
}

pub(crate) fn session_meta(r: &rusqlite::Row<'_>) -> rusqlite::Result<SessionMeta> {
    Ok(SessionMeta {
        id: r.get(0)?,
        name: r.get(1)?,
        started_at: r.get(2)?,
        cwd: r.get(3)?,
        provider: r.get(4)?,
        model: r.get(5)?,
        anchor: r.get(6)?,
        meta: r.get(7)?,
        todo_on: r.get::<_, i64>(8)? != 0,
    })
}

/// One block row, in [`BLOCK_COLS`] order.
pub(crate) fn block_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Block> {
    let raw: String = r.get(1)?;
    let kind = BlockKind::parse(&raw).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, e.into())
    })?;
    Ok(Block {
        id: r.get(0)?,
        kind,
        content: r.get(2)?,
        prefix: r.get(3)?,
        suffix: r.get(4)?,
        usage_total: count(r, 5)?,
        usage_prompt: count(r, 6)?,
        usage_completion: count(r, 7)?,
        usage_cached: count(r, 8)?,
        usage_cache_write: count(r, 9)?,
        usage_reasoning: count(r, 10)?,
        call_id: r.get(11)?,
        name: r.get(12)?,
        args: r.get(13)?,
        intent: r.get(14)?,
        text: r.get(15)?,
        first: flag(r, 16)?,
        ok: flag(r, 17)?,
        result: r.get(18)?,
        memo: r.get(19)?,
        duration_ms: r.get(20)?,
        ts: r.get(21)?,
    })
}

/// A nullable integer column as an unsigned count.
fn count(r: &rusqlite::Row<'_>, i: usize) -> rusqlite::Result<Option<u64>> {
    Ok(r.get::<_, Option<i64>>(i)?.map(|v| v.max(0) as u64))
}

/// A nullable 0/1 column as a flag.
fn flag(r: &rusqlite::Row<'_>, i: usize) -> rusqlite::Result<Option<bool>> {
    Ok(r.get::<_, Option<i64>>(i)?.map(|v| v != 0))
}

pub(crate) fn turn_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<TurnRow> {
    let blocks: String = r.get(1)?;
    let blocks = parse_blocks(&blocks).unwrap_or_default();
    // The four token columns live and die together: probe the prompt column
    // and backfill zeros — no "half a usage" state exists.
    let prompt: Option<i64> = r.get(3)?;
    let usage = prompt.map(|p| RoundUsage {
        prompt_tokens: p.max(0) as u64,
        completion_tokens: r
            .get::<_, Option<i64>>(4)
            .unwrap_or(None)
            .unwrap_or(0)
            .max(0) as u64,
        cached_tokens: r
            .get::<_, Option<i64>>(5)
            .unwrap_or(None)
            .unwrap_or(0)
            .max(0) as u64,
        cache_write_tokens: r
            .get::<_, Option<i64>>(6)
            .unwrap_or(None)
            .unwrap_or(0)
            .max(0) as u64,
    });
    Ok(TurnRow {
        turn_id: r.get(0)?,
        blocks,
        stop_reason: r.get(2)?,
        usage,
    })
}

// ---- SQL column lists shared by the readers ----

/// A block row's columns, in [`block_from_row`] read order.
pub(crate) const BLOCK_COLS: &str = "block_id, type, content, prefix, suffix,
     usage_total, usage_prompt, usage_completion, usage_cached, usage_cache_write,
     usage_reasoning, call_id, name, args, intent, text, first, ok, result, memo,
     duration_ms, ts";

/// A session row's columns, in [`session_meta`] read order.
pub(crate) const SESSION_COLS: &str =
    "id, name, started_at, cwd, provider, model, anchor, meta, todo_on";

/// A request archive row's columns, in [`request_from_row`] read order.
pub(crate) const REQUEST_COLS: &str =
    "request_id, hash, model, provider, system, tools_json, baseline_prompt";

/// One request archive row: its id, its hash, and the archive itself.
pub(crate) fn request_from_row(
    r: &rusqlite::Row<'_>,
) -> rusqlite::Result<(i64, String, RequestMeta)> {
    Ok((
        r.get(0)?,
        r.get(1)?,
        RequestMeta {
            model: r.get(2)?,
            provider: r.get(3)?,
            system: r.get(4)?,
            tools_json: r.get(5)?,
            baseline_prompt: r.get::<_, Option<i64>>(6)?.map(|v| v.max(0) as u64),
        },
    ))
}

/// A round row's columns, in [`turn_from_row`] read order.
pub(crate) const TURN_COLS: &str = "t.turn_id, t.blocks, t.stop_reason,
     t.prompt_tokens, t.completion_tokens, t.cached_tokens, t.cache_write_tokens";

/// The round table, aliased `t`.
pub(crate) const TURN_FROM: &str = "FROM turns t";
