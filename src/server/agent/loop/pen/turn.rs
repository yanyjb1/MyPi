//! The rows one turn opens and closes with.

use std::path::Path;

use anyhow::Result;

use crate::server::agent::r#loop::session::Retention;
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
        /// The rendered instruction; the round keeps it as its own record of this compaction.
        instruction: String,
    },
    Wake {
        text: String,
    },
}

/// Open one turn: its row, its list place, its working directory, and its opening block when it
/// has one. A compaction opens with no block: its instruction and its summary land together when
/// it finishes, so a compaction that fails leaves nothing for the next request to read.
pub fn open(
    w: &Writer,
    session: i64,
    start: Start,
    cwd: &Path,
    model: Option<(String, String)>,
) -> Result<(i64, Option<Stored>)> {
    let (book, message) = match start {
        Start::User {
            prefix,
            text,
            suffix,
        } => (
            Some(NewBlock {
                kind: BlockKind::User,
                content: Some(text.clone()),
                prefix: prefix.clone(),
                suffix: suffix.clone(),
                ts: Some(crate::server::platform::time::now_stamp()),
                ..NewBlock::default()
            }),
            Some(Stored::User {
                prefix,
                content: text,
                suffix,
            }),
        ),
        Start::Compact { .. } => (None, None),
        Start::Wake { text } => (
            Some(NewBlock {
                kind: BlockKind::Wake,
                content: Some(text.clone()),
                ts: Some(crate::server::platform::time::now_stamp()),
                ..NewBlock::default()
            }),
            Some(Stored::User {
                prefix: None,
                content: text,
                suffix: None,
            }),
        ),
    };
    let mut rows = vec![Row::Turn {
        stop_reason: None,
        model,
    }];
    if let Some(block) = book {
        rows.push(Row::Block {
            turn: Ref::Batch(0),
            block,
        });
    }
    rows.push(Row::List {
        session,
        turn: Ref::Batch(0),
    });
    rows.push(Row::Cwd {
        session,
        turn: Ref::Batch(0),
        cwd: cwd.to_string_lossy().into_owned(),
    });
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
pub fn append_user(
    w: &Writer,
    turn: i64,
    prefix: Option<String>,
    text: String,
    suffix: Option<String>,
) -> Result<()> {
    let block = NewBlock {
        kind: BlockKind::User,
        content: Some(text),
        prefix,
        suffix,
        ts: Some(crate::server::platform::time::now_stamp()),
        ..NewBlock::default()
    };
    w.push(Submit {
        rows: vec![Row::Block {
            turn: Ref::Known(turn),
            block,
        }],
    })?;
    Ok(())
}

/// Land a finished compaction in one submission: the instruction it sent, its summary, the rounds
/// it borrowed back, and the mark that makes this round the start of the history. Either all of it
/// lands or none does, so a compaction that breaks leaves no trace.
pub fn land_compaction(
    w: &Writer,
    session: i64,
    turn: i64,
    instruction: String,
    text: String,
    usage: Usage,
    borrows: &[crate::server::agent::r#loop::turn::ask::Borrow],
) -> Result<()> {
    let mut rows = vec![
        Row::Block {
            turn: Ref::Known(turn),
            block: NewBlock {
                kind: BlockKind::User,
                content: Some(instruction),
                ts: Some(crate::server::platform::time::now_stamp()),
                ..NewBlock::default()
            },
        },
        Row::Block {
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
        },
    ];
    for borrow in borrows {
        let at = rows.len();
        rows.push(Row::Turn {
            stop_reason: Some("borrowed".to_string()),
            model: None,
        });
        rows.push(Row::Borrow {
            turn: Ref::Batch(at),
            ranges: borrow.ranges.clone(),
        });
        rows.push(Row::List {
            session,
            turn: Ref::Batch(at),
        });
    }
    rows.push(Row::Compact {
        session,
        turn: Ref::Known(turn),
    });
    w.push(Submit { rows })?;
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
