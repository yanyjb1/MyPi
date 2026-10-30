//! The rows one turn opens and closes with.

use std::path::Path;

use anyhow::Result;

use crate::server::agent::r#loop::session::{Message, Retention};
use crate::server::agent::r#loop::turn::ask::{Stop, Usage};
use crate::server::storage::{BlockKind, NewBlock, Ref, RoundUsage, Row, Submit, Writer};

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
pub fn open(w: &Writer, session: i64, start: Start, cwd: &Path) -> Result<i64> {
    let (block, compaction) = match start {
        Start::User {
            prefix,
            text,
            suffix,
        } => (
            NewBlock {
                kind: BlockKind::User,
                content: Some(text),
                prefix,
                suffix,
                ts: Some(crate::server::platform::time::now_stamp()),
                ..NewBlock::default()
            },
            false,
        ),
        Start::Compact { focus, .. } => (
            NewBlock {
                kind: BlockKind::User,
                content: Some(focus.unwrap_or_default()),
                ts: Some(crate::server::platform::time::now_stamp()),
                ..NewBlock::default()
            },
            true,
        ),
        Start::Wake { text } => (
            NewBlock {
                kind: BlockKind::Wake,
                content: Some(text),
                ts: Some(crate::server::platform::time::now_stamp()),
                ..NewBlock::default()
            },
            false,
        ),
    };
    let rows = vec![
        Row::Turn {
            stop_reason: None,
            usage: None,
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
    landed
        .ids
        .first()
        .copied()
        .filter(|id| *id > 0)
        .ok_or_else(|| anyhow::anyhow!("the writer opened no turn"))
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

/// Append the compacted summary to a running turn.
pub fn append_summary(w: &Writer, turn: i64, text: String) -> Result<()> {
    w.push(Submit {
        rows: vec![Row::Block {
            turn: Ref::Known(turn),
            block: NewBlock {
                kind: BlockKind::Assistant,
                content: Some(text),
                ts: Some(crate::server::platform::time::now_stamp()),
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
            usage: None,
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

/// Close one turn and lower the archive's prompt baseline.
pub fn settle(
    w: &Writer,
    turn: i64,
    stop: Stop,
    usage: Usage,
    baseline: Option<u64>,
    meta: Option<i64>,
) -> Result<()> {
    let round = RoundUsage {
        prompt_tokens: usage.prompt,
        completion_tokens: usage.completion,
        cached_tokens: usage.cached,
        cache_write_tokens: usage.cache_write,
    };
    let mut rows = vec![Row::Settle {
        turn: Ref::Known(turn),
        stop_reason: Some(stop.as_str().to_string()),
        usage: Some(round),
    }];
    if let (Some(meta), Some(prompt)) = (meta, baseline.filter(|p| *p > 0)) {
        rows.push(Row::Baseline {
            request: Ref::Known(meta),
            prompt,
        });
    }
    w.push(Submit { rows })?;
    Ok(())
}
