//! The request half's block write end: one block, plus the artifact it spilled into.

use anyhow::Result;

use crate::server::agent::request::contract::{Artifact, Block, BlockSink, Kind};
use crate::server::storage::{BlockKind, NewBlock, Ref, Row, Submit, Writer};

/// One turn's block writer.
pub struct Pen {
    writer: Writer,
    turn: i64,
}

impl Pen {
    pub fn new(writer: Writer, turn: i64) -> Self {
        Self { writer, turn }
    }
}

impl BlockSink for Pen {
    fn land(&self, block: Block, artifact: Option<Artifact>) -> Result<()> {
        let mut rows = vec![Row::Block {
            turn: Ref::Known(self.turn),
            block: land(block),
        }];
        if let Some(artifact) = artifact {
            rows.extend(super::spill::rows(Ref::Batch(0), artifact));
        }
        self.writer.push(Submit { rows })?;
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
