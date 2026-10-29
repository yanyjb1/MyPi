//! Decoding: a database row becomes a [`shape`](super::shape) value. Storage is the only place that
//! knows the column layout, so callers name columns through the constants here.

use anyhow::{Context, Result};

use super::shape::{Artifact, Block, BlockKind, RequestMeta, RoundUsage, SessionMeta, TurnRow};

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
        model_provider: r.get(4)?,
        model: r.get(5)?,
        profile: r.get(6)?,
        todo_on: r.get::<_, i64>(7)? != 0,
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

/// One artifact row, in [`ARTIFACT_COLS`] order. Its block is its id: one
/// block spills at most one artifact.
pub(crate) fn artifact_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Artifact> {
    Ok(Artifact {
        block: r.get(0)?,
        name: r.get(1)?,
        total_lines: r.get(2)?,
        content: r.get(3)?,
    })
}

pub(crate) fn turn_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<TurnRow> {
    let blocks: String = r.get(2)?;
    let blocks = parse_blocks(&blocks).unwrap_or_default();
    // The three token columns live and die together: probe the prompt column
    // and backfill zeros — no "half a usage" state exists.
    let prompt: Option<i64> = r.get(5)?;
    let usage = prompt.map(|p| RoundUsage {
        prompt_tokens: p.max(0) as u64,
        completion_tokens: r
            .get::<_, Option<i64>>(6)
            .unwrap_or(None)
            .unwrap_or(0)
            .max(0) as u64,
        cached_tokens: r
            .get::<_, Option<i64>>(7)
            .unwrap_or(None)
            .unwrap_or(0)
            .max(0) as u64,
        cache_write_tokens: r
            .get::<_, Option<i64>>(8)
            .unwrap_or(None)
            .unwrap_or(0)
            .max(0) as u64,
    });
    let request_id: Option<i64> = r.get(1)?;
    let request = match (request_id, r.get::<_, Option<String>>(10)?) {
        (Some(id), Some(model)) => {
            let _ = id;
            Some(RequestMeta {
                model,
                provider: r.get::<_, Option<String>>(11)?.unwrap_or_default(),
                protocol: r.get::<_, Option<String>>(12)?.unwrap_or_default(),
                base_url: r.get::<_, Option<String>>(13)?.unwrap_or_default(),
                profile: r.get::<_, Option<String>>(18)?.unwrap_or_default(),
                system: r.get::<_, Option<String>>(14)?.unwrap_or_default(),
                tools_json: r.get::<_, Option<String>>(15)?.unwrap_or_default(),
                max_tokens: r.get::<_, Option<i64>>(16)?.unwrap_or(0).max(0) as u32,
                compat_json: r.get(17)?,
            })
        }
        _ => None,
    };
    Ok(TurnRow {
        turn_id: r.get(0)?,
        request_id,
        blocks,
        stop_reason: r.get(3)?,
        bytes: r.get(4)?,
        usage,
        request,
    })
}

// ---- SQL column lists shared by the readers ----

/// A block row's columns, in [`block_from_row`] read order.
pub(crate) const BLOCK_COLS: &str = "block_id, type, content, prefix, suffix,
     usage_total, usage_prompt, usage_completion, usage_cached, usage_cache_write,
     usage_reasoning, call_id, name, args, intent, text, first, ok, result, memo,
     duration_ms, ts";

/// An artifact row's columns, in [`artifact_from_row`] read order.
pub(crate) const ARTIFACT_COLS: &str = "block_id, name, total_lines, content";

/// A block's byte volume, for the queries that sum it (the write side mirrors
/// this in `write/batches.rs::bytes_of` — keep the two in step).
pub(crate) const BLOCK_BYTES_SQL: &str =
    "LENGTH(COALESCE(content, '')) + LENGTH(COALESCE(prefix, ''))
     + LENGTH(COALESCE(suffix, '')) + LENGTH(COALESCE(call_id, '')) + LENGTH(COALESCE(name, ''))
     + LENGTH(COALESCE(args, '')) + LENGTH(COALESCE(intent, '')) + LENGTH(COALESCE(text, ''))
     + LENGTH(COALESCE(result, '')) + LENGTH(COALESCE(memo, '')) + LENGTH(COALESCE(ts, ''))";

/// Columns of a round row plus its joined request archive, in
/// [`turn_from_row`] read order.
pub(crate) const TURN_COLS: &str = "t.turn_id, t.request_id, t.blocks, t.stop_reason, t.bytes,
     t.prompt_tokens, t.completion_tokens, t.cached_tokens,
     t.cache_write_tokens,
     r.hash, r.model, r.provider, r.protocol, r.base_url, r.system,
     r.tools_json, r.max_tokens, r.compat_json, r.profile";

/// Round rows left-joined to their request archive.
pub(crate) const TURN_FROM: &str =
    "FROM turns t LEFT JOIN request_meta r ON r.request_id = t.request_id";
