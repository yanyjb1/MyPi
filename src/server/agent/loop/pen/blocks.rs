//! The request half's block write end.

use anyhow::Result;

use crate::server::agent::r#loop::turn::ask::{Artifact, BlockSink};
use crate::server::storage::{NewBlock, Ref, Row, Submit, Writer};

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
    fn block(&self, mut block: NewBlock, artifact: Option<Artifact>) -> Result<()> {
        if block.ts.is_none() {
            block.ts = Some(crate::server::platform::time::now_stamp());
        }
        let mut rows = vec![Row::Block {
            turn: Ref::Known(self.turn),
            block,
        }];
        if let Some(artifact) = artifact {
            rows.extend(super::spill::rows(
                Ref::Known(self.turn),
                Ref::Batch(0),
                artifact,
            ));
        }
        self.writer.push(Submit { rows })?;
        Ok(())
    }
}
