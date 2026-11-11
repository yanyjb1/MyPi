//! The rows one turn opens and closes with.

use std::path::Path;

use anyhow::Result;

use crate::server::agent::r#loop::session::{Message, Retention};
use crate::server::agent::r#loop::turn::ask::{Stop, Usage};
use crate::server::agent::request::message::Message as Stored;
use crate::server::storage::{BlockKind, NewBlock, Ref, Row, Structure, Submit, Writer};

/// How a turn opens.
#[derive(Clone)]
pub enum Start {
    User {
        prefix: Option<String>,
        text: String,
        suffix: Option<String>,
    },
    Compact {
        retention: Option<Retention>,
        focus: Option<String>,
    },
    Wake {
        text: String,
    },
}

/// Open one turn: its row, its first block, its list place, its working directory.
pub fn open(
    w: &Writer,
    session: i64,
    start: Start,
    cwd: &Path,
    model: Option<(String, String)>,
) -> Result<(i64, Stored)> {
    let (block, compaction, message) = match start {
        Start::User {
            prefix,
            text,
            suffix,
        } => (
            NewBlock {
                kind: BlockKind::User,
                content: Some(text.clone()),
                prefix: prefix.clone(),
                suffix: suffix.clone(),
                ts: Some(crate::server::platform::time::now_stamp()),
                ..NewBlock::default()
            },
            false,
            Stored::User {
                prefix,
                content: text,
                suffix,
            },
        ),
        Start::Compact { focus, .. } => {
            let content = focus.unwrap_or_default();
            (
                NewBlock {
                    kind: BlockKind::User,
                    content: Some(content.clone()),
                    ts: Some(crate::server::platform::time::now_stamp()),
                    ..NewBlock::default()
                },
                true,
                Stored::User {
                    prefix: None,
                    content,
                    suffix: None,
                },
            )
        }
        Start::Wake { text } => (
            NewBlock {
                kind: BlockKind::Wake,
                content: Some(text.clone()),
                ts: Some(crate::server::platform::time::now_stamp()),
                ..NewBlock::default()
            },
            false,
            Stored::User {
                prefix: None,
                content: text,
                suffix: None,
            },
        ),
    };
    let rows = vec![
        Row::Turn {
            stop_reason: None,
            model,
        },
        Row::Block {
            turn: Ref::Batch(0),
            block,
        },
        Row::List {
            session,
            turn: Ref::Batch(0),
            is_compaction: compaction,
        },
        Row::Cwd {
            session,
            turn: Ref::Batch(0),
            cwd: cwd.to_string_lossy().into_owned(),
        },
    ];
    let landed = w.push(Submit { rows })?;
    let turn = landed
        .ids
        .first()
        .copied()
        .filter(|id| *id > 0)
        .ok_or_else(|| anyhow::anyhow!("the writer opened no turn"))?;
    Ok((turn, message))
}

/// Append one user message to a running turn.
pub fn append_user(w: &Writer, turn: i64, msg: Message) -> Result<()> {
    let block = match msg {
        Message::Submit {
            prefix,
            text,
            suffix,
        } => NewBlock {
            kind: BlockKind::User,
            content: Some(text),
            prefix,
            suffix,
            ts: Some(crate::server::platform::time::now_stamp()),
            ..NewBlock::default()
        },
        Message::Compact { focus, .. } => NewBlock {
            kind: BlockKind::User,
            content: Some(focus.unwrap_or_default()),
            ts: Some(crate::server::platform::time::now_stamp()),
            ..NewBlock::default()
        },
    };
    w.push(Submit {
        rows: vec![Row::Block {
            turn: Ref::Known(turn),
            block,
        }],
    })?;
    Ok(())
}

/// Append the compacted summary to a running turn, carrying the compaction's bill.
pub fn append_summary(w: &Writer, turn: i64, text: String, usage: Usage) -> Result<()> {
    w.push(Submit {
        rows: vec![Row::Block {
            turn: Ref::Known(turn),
            block: NewBlock {
                kind: BlockKind::Assistant,
                content: Some(text),
                ts: Some(crate::server::platform::time::now_stamp()),
                usage_total: Some(usage.prompt + usage.completion),
                usage_prompt: Some(usage.prompt),
                usage_completion: Some(usage.completion),
                usage_cached: Some(usage.cached),
                usage_cache_write: Some(usage.cache_write),
                usage_reasoning: Some(usage.reasoning),
                ..NewBlock::default()
            },
        }],
    })?;
    Ok(())
}

/// Append one wake block to a running turn.
pub fn append_wake(w: &Writer, turn: i64, text: String) -> Result<()> {
    w.push(Submit {
        rows: vec![Row::Block {
            turn: Ref::Known(turn),
            block: NewBlock {
                kind: BlockKind::Wake,
                content: Some(text),
                ts: Some(crate::server::platform::time::now_stamp()),
                ..NewBlock::default()
            },
        }],
    })?;
    Ok(())
}

/// Append the retained intervals as borrowed rounds after the compaction round.
pub fn borrow(
    w: &Writer,
    session: i64,
    borrows: &[crate::server::agent::r#loop::turn::ask::Borrow],
) -> Result<()> {
    let mut rows = Vec::new();
    for borrow in borrows {
        let turn = rows.len();
        rows.push(Row::Turn {
            stop_reason: Some("borrowed".to_string()),
            model: None,
        });
        rows.push(Row::Borrow {
            turn: Ref::Batch(turn),
            ranges: borrow.ranges.clone(),
        });
        rows.push(Row::List {
            session,
            turn: Ref::Batch(turn),
            is_compaction: false,
        });
    }
    if !rows.is_empty() {
        w.push(Submit { rows })?;
    }
    Ok(())
}

/// Close one turn and lower the archive's prompt baseline; the closed ranges come back for the
/// live line, so the receipt travels the same queue as everything else this turn said.
pub fn settle(
    w: &Writer,
    turn: i64,
    stop: Stop,
    baseline: Option<u64>,
    meta: Option<i64>,
) -> Result<Option<Structure>> {
    let mut rows = vec![Row::Settle {
        turn: Ref::Known(turn),
        stop_reason: Some(stop.as_str().to_string()),
    }];
    if let (Some(meta), Some(prompt)) = (meta, baseline.filter(|p| *p > 0)) {
        rows.push(Row::Baseline {
            request: Ref::Known(meta),
            prompt,
        });
    }
    Ok(w.push(Submit { rows })?.pieces.into_iter().next())
}
