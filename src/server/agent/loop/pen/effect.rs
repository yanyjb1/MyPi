//! Tool effects, turned into rows anchored on the result block that produced them.

use anyhow::Result;

use crate::server::agent::tools::contract::output::{AliasOp, Effect, MemoOp};
use crate::server::storage::{Ref, Row, Table, Writer};

/// A row that must be resolved by name before it can be deleted.
pub enum Delete {
    Memo(String),
    Alias(String),
}

/// One effect → its rows, plus the deletion it asks for, if any.
pub fn rows(anchor: Ref, effect: &Effect) -> (Vec<Row>, Option<Delete>) {
    match effect {
        Effect::Memo {
            op,
            name,
            content,
            ts,
        } => match op {
            MemoOp::Set => (
                vec![Row::Memo {
                    anchor,
                    name: name.clone(),
                    content: content.clone().unwrap_or_default(),
                    ts: ts.clone(),
                }],
                None,
            ),
            MemoOp::Delete => (Vec::new(), Some(Delete::Memo(name.clone()))),
        },
        Effect::Alias {
            op,
            name,
            command,
            ts,
        } => match op {
            AliasOp::Add => (
                vec![Row::Alias {
                    anchor,
                    name: name.clone(),
                    command: command.clone().unwrap_or_default(),
                    ts: ts.clone(),
                }],
                None,
            ),
            AliasOp::Delete => (Vec::new(), Some(Delete::Alias(name.clone()))),
        },
        Effect::Child { .. } => (Vec::new(), None),
    }
}

/// Resolve the names to ids inside the session's visible blocks, then delete them.
pub fn apply(writer: &Writer, session: i64, deletes: &[Delete]) -> Result<()> {
    use crate::server::storage::query::{Cond, Query, Sheet, Val};

    let reader = crate::server::storage::Reader::open(&writer.path())?;
    let ranges = super::super::session::source::ranges(&reader, session)?;
    for delete in deletes {
        let (sheet, table, name) = match delete {
            Delete::Memo(name) => (Sheet::Memos, Table::Memos, name),
            Delete::Alias(name) => (Sheet::Aliases, Table::Aliases, name),
        };
        let rows = reader.query(&Query::new(sheet).columns(&["id"]).filter(vec![
            Cond::Within("anchor", ranges.clone()),
            Cond::Eq("name", Val::Text(name.clone())),
        ]))?;
        let ids: Vec<i64> = rows.iter().filter_map(|row| row["id"].as_i64()).collect();
        if !ids.is_empty() {
            writer.delete_rows(table, &ids)?;
        }
    }
    Ok(())
}
