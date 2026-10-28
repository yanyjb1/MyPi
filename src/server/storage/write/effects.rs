//! Side effects carried by a tool result: memo, alias, subsession.

use anyhow::Result;
use rusqlite::params;

use super::super::shape::{Effect, EffectKind};
use super::{aliases, memos};

/// Apply one effect after its tool result block has an id.
pub(crate) fn apply(
    tx: &rusqlite::Transaction<'_>,
    session: i64,
    block_id: i64,
    effect: &Effect,
) -> Result<()> {
    match &effect.kind {
        EffectKind::Memo {
            op,
            scope,
            anchor,
            name,
            content,
        } => memos::apply(
            tx,
            session,
            block_id,
            *op,
            *scope,
            *anchor,
            name,
            content.as_deref(),
            &effect.ts,
        ),
        EffectKind::Alias { op, name, command } => aliases::apply(
            tx,
            session,
            block_id,
            *op,
            name,
            command.as_deref(),
            &effect.ts,
        ),
        EffectKind::Subsession { session: child } => {
            tx.execute(
                "INSERT INTO subsessions (subsession_id, anchor) VALUES (?1, ?2)",
                params![child, block_id],
            )?;
            Ok(())
        }
    }
}
