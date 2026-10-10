//! Batches: the one write path into the database. A round exists once its first batch lands, and
//! the **when** belongs to the writer's inlet — this module is the **how**.

use anyhow::Result;

use super::shared::{append_list_row, put_request_meta, read_turn};
use super::super::decode::encode_blocks;
use super::super::shape::{Block, NewBlock, RequestMeta, RoundUsage, base36};
use super::super::writer::Store;
use super::artifacts::put_artifact;

impl Store {
    /// Open a round **and land its first batch**, in one transaction.
    pub(crate) fn open_turn(
        &mut self,
        session_id: i64,
        blocks: &[NewBlock],
        request: Option<&RequestMeta>,
    ) -> Result<(i64, Vec<Block>)> {
        let tx = self.conn.transaction()?;
        let request_id = match request {
            Some(r) => Some(put_request_meta(&tx, r)?),
            None => None,
        };
        tx.execute(
            "INSERT INTO turns (request_id) VALUES (?1)",
            rusqlite::params![request_id],
        )?;
        let turn_id = tx.last_insert_rowid();
        append_list_row(&tx, session_id, turn_id, false)?;
        let (range, bytes, landed) = land(&tx, blocks)?;
        if let Some(range) = range {
            tx.execute(
                "UPDATE turns SET blocks = ?1, bytes = ?2 WHERE turn_id = ?3",
                rusqlite::params![encode_blocks(&[range]), bytes, turn_id],
            )?;
        }
        tx.commit()?;
        Ok((turn_id, landed))
    }

    /// Land **one more batch** of blocks: insert them, append the id interval to the round's block
    /// index, add the bytes. One transaction, so the ids are consecutive.
    pub(crate) fn append_blocks(
        &mut self,
        turn_id: i64,
        blocks: &[NewBlock],
    ) -> Result<Vec<Block>> {
        if blocks.is_empty() {
            return Ok(Vec::new());
        }
        let tx = self.conn.transaction()?;
        let (range, bytes, landed) = land(&tx, blocks)?;
        let range = range.expect("blocks is not empty");
        let turn = read_turn(&tx, turn_id)?
            .ok_or_else(|| anyhow::anyhow!("round {turn_id} does not exist"))?;
        let mut index = turn.blocks;
        index.push(range);
        tx.execute(
            "UPDATE turns SET blocks = ?1, bytes = bytes + ?2 WHERE turn_id = ?3",
            rusqlite::params![encode_blocks(&index), bytes, turn_id],
        )?;
        tx.commit()?;
        Ok(landed)
    }

    /// Attach the request archive to an already-open round row. Archives are deduplicated by
    /// content: one row per distinct archive.
    pub(crate) fn attach_request(&mut self, turn_id: i64, request: &RequestMeta) -> Result<()> {
        let tx = self.conn.transaction()?;
        let id = put_request_meta(&tx, request)?;
        tx.execute(
            "UPDATE turns SET request_id = ?1 WHERE turn_id = ?2",
            rusqlite::params![id, turn_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Settle a round: fill in the stop reason and usage (`None` = the gateway
    /// never answered / never reported).
    pub(crate) fn finish_turn(
        &mut self,
        turn_id: i64,
        stop_reason: Option<&str>,
        usage: Option<RoundUsage>,
    ) -> Result<()> {
        let (p, c, k, r, w) = match usage {
            Some(u) => (
                Some(u.prompt_tokens as i64),
                Some(u.completion_tokens as i64),
                Some(u.cached_tokens as i64),
                Some(u.reasoning_tokens as i64),
                Some(u.cache_write_tokens as i64),
            ),
            None => (None, None, None, None, None),
        };
        self.conn.execute(
            "UPDATE turns SET stop_reason = ?1, prompt_tokens = ?2,
                    completion_tokens = ?3, cached_tokens = ?4, reasoning_tokens = ?5,
                    cache_write_tokens = ?6
              WHERE turn_id = ?7",
            rusqlite::params![stop_reason, p, c, k, r, w, turn_id],
        )?;
        Ok(())
    }
}

/// Write one batch's blocks and report the interval they occupy, the bytes they weigh, and the
/// blocks themselves. A spilled block lands its artifact in the same transaction.
pub(super) fn land(
    tx: &rusqlite::Transaction<'_>,
    blocks: &[NewBlock],
) -> Result<(Option<[i64; 2]>, i64, Vec<Block>)> {
    let mut landed = Vec::with_capacity(blocks.len());
    let mut bytes = 0i64;
    let mut range: Option<[i64; 2]> = None;
    for b in blocks {
        bytes += bytes_of(b);
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
        let mut landed_block = b.clone().landed(id);
        if let Some(spill) = &b.spill {
            put_artifact(tx, id, &spill.name, &spill.content)?;
            let text = landed_block.result.take().unwrap_or_default();
            let cited = cite(&text, id);
            bytes += (cited.len() - text.len()) as i64;
            tx.execute(
                "UPDATE blocks SET result = ?1 WHERE block_id = ?2",
                rusqlite::params![cited, id],
            )?;
            landed_block.result = Some(cited);
        }
        landed.push(landed_block);
        match &mut range {
            None => range = Some([id, id]),
            Some(r) => r[1] = id,
        }
    }
    Ok((range, bytes, landed))
}

/// The result text of a block that spilled: the visible stretch, then the citation.
fn cite(text: &str, id: i64) -> String {
    let r = base36(id);
    let line = format!("[full output: #{r} — read it with bash (`cat #{r}`) or context (`#{r}`)]");
    if text.is_empty() {
        line
    } else {
        format!("{text}\n{line}")
    }
}

/// A block's byte volume: the lengths of its text columns. The SQL side of
/// the same measure is `decode::BLOCK_BYTES_SQL` — keep the two in step.
pub(crate) fn bytes_of(b: &NewBlock) -> i64 {
    let mut n = 0i64;
    for s in [
        &b.content, &b.prefix, &b.suffix, &b.call_id, &b.name, &b.args, &b.intent, &b.text,
        &b.result, &b.memo, &b.ts,
    ] {
        if let Some(s) = s {
            n += s.len() as i64;
        }
    }
    n
}
