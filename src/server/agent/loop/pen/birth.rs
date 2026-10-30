//! The rows a child session is born with.

use anyhow::Result;

use crate::server::storage::{Ref, Row, Submit, Writer};

/// Write a child session row. The caller supplies the spawn block; the row is
/// the only place that link lives.
#[allow(clippy::too_many_arguments)]
pub fn child(
    w: &Writer,
    started_at: &str,
    cwd: &str,
    provider: &str,
    model: &str,
    anchor: Ref,
    meta: Option<Ref>,
) -> Result<i64> {
    let landed = w.push(Submit {
        rows: vec![Row::Session {
            started_at: started_at.to_string(),
            cwd: cwd.to_string(),
            model: Some((provider.to_string(), model.to_string())),
            name: None,
            anchor: Some(anchor),
            meta,
        }],
    })?;
    landed
        .ids
        .first()
        .copied()
        .filter(|id| *id > 0)
        .ok_or_else(|| anyhow::anyhow!("the writer created no child session"))
}
