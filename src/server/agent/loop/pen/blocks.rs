//! The request half's block write end: one lap's blocks and their artifacts, landed together.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::Result;

use crate::server::agent::request::contract::{Artifact, Block, BlockSink, Kind};
use crate::server::storage::{BlockKind, Landed, NewBlock, Ref, Row, Submit, Writer};

/// One turn's block writer. Rows are held until the lap ends, so effects can join the same commit.
pub struct Pen {
    writer: Writer,
    turn: i64,
    rows: Mutex<Vec<Row>>,
    at: Mutex<HashMap<String, usize>>,
}

impl Pen {
    pub fn new(writer: Writer, turn: i64) -> Self {
        Self {
            writer,
            turn,
            rows: Mutex::new(Vec::new()),
            at: Mutex::new(HashMap::new()),
        }
    }

    /// Where a call's result block sits in the next commit.
    pub fn anchor(&self, call_id: &str) -> Option<Ref> {
        self.at
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(call_id)
            .map(|index| Ref::Batch(*index))
    }

    /// Where the next pushed row sits.
    pub fn len(&self) -> usize {
        self.rows.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn push(&self, rows: Vec<Row>) {
        self.rows
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend(rows);
    }

    /// Write everything held so far as one submission.
    pub fn flush(&self) -> Result<Landed> {
        let rows = std::mem::take(&mut *self.rows.lock().unwrap_or_else(|e| e.into_inner()));
        if rows.is_empty() {
            return Ok(Landed::default());
        }
        self.writer.push(Submit { rows })
    }
}

impl BlockSink for Pen {
    fn land(&self, block: Block, artifact: Option<Artifact>) -> Result<()> {
        let mut rows = self.rows.lock().unwrap_or_else(|e| e.into_inner());
        let at = rows.len();
        if let Some(call_id) = block.call_id.clone() {
            self.at
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(call_id, at);
        }
        rows.push(Row::Block {
            turn: Ref::Known(self.turn),
            block: land(block),
        });
        if let Some(artifact) = artifact {
            rows.extend(super::spill::rows(Ref::Batch(at), artifact));
        }
        Ok(())
    }
}

/// The request half's block as a storage row.
fn land(block: Block) -> NewBlock {
    NewBlock {
        kind: kind(block.kind),
        content: block.content,
        prefix: None,
        suffix: None,
        usage_total: block.usage.map(|u| u.prompt + u.completion),
        usage_prompt: block.usage.map(|u| u.prompt),
        usage_completion: block.usage.map(|u| u.completion),
        usage_cached: block.usage.map(|u| u.cached),
        usage_cache_write: block.usage.map(|u| u.cache_write),
        usage_reasoning: block.usage.map(|u| u.reasoning),
        call_id: block.call_id,
        name: block.name,
        args: block.args,
        intent: block.intent,
        text: None,
        first: block.first,
        ok: block.ok,
        result: block.result,
        memo: block.memo,
        duration_ms: block.duration_ms.map(|ms| ms as i64),
        ts: block.ts,
    }
}

fn kind(kind: Kind) -> BlockKind {
    match kind {
        Kind::Assistant => BlockKind::Assistant,
        Kind::Reasoning => BlockKind::Reasoning,
        Kind::ToolRequest => BlockKind::ToolRequest,
        Kind::ToolResult => BlockKind::ToolResult,
    }
}
