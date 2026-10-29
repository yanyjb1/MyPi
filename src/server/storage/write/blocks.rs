//! Block rows: one insert, and the round's interval that follows from it.

use anyhow::Result;

use super::super::decode::{encode_blocks, parse_blocks};
use super::super::shape::{Block, NewBlock};

/// One block row; its id joins its round's interval.
pub(crate) fn insert(tx: &rusqlite::Transaction<'_>, turn: i64, b: &NewBlock) -> Result<Block> {
    tx.execute(
        "INSERT INTO blocks (type, content, prefix, suffix,
                usage_total, usage_prompt, usage_completion, usage_cached,
                usage_cache_write, usage_reasoning, call_id, name, args, intent,
                text, first, ok, result, memo, duration_ms, ts)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                 ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)",
        rusqlite::params![
            b.kind.as_str(),
            b.content,
            b.prefix,
            b.suffix,
            b.usage_total.map(|v| v as i64),
            b.usage_prompt.map(|v| v as i64),
            b.usage_completion.map(|v| v as i64),
            b.usage_cached.map(|v| v as i64),
            b.usage_cache_write.map(|v| v as i64),
            b.usage_reasoning.map(|v| v as i64),
            b.call_id,
            b.name,
            b.args,
            b.intent,
            b.text,
            b.first.map(|v| v as i64),
            b.ok.map(|v| v as i64),
            b.result,
            b.memo,
            b.duration_ms,
            b.ts,
        ],
    )?;
    let id = tx.last_insert_rowid();
    let raw: String = tx.query_row("SELECT blocks FROM turns WHERE turn_id = ?1", [turn], |r| {
        r.get(0)
    })?;
    let mut index = parse_blocks(&raw)?;
    index.push([id, id]);
    tx.execute(
        "UPDATE turns SET blocks = ?1 WHERE turn_id = ?2",
        rusqlite::params![encode_blocks(&index), turn],
    )?;
    Ok(b.clone().landed(id))
}
