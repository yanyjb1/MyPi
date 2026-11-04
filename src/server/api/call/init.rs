//! `init` — the whole structure, plus the newest blocks.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;

const LATEST: usize = 32;

pub(crate) fn answer(api: &Api, session: i64) -> Result<Value, Refuse> {
    let reader = Reader::open(api.db())?;
    let list = reader.list(session)?;
    let from = list
        .iter()
        .rev()
        .find(|row| row["is_compaction"].as_bool() == Some(true))
        .and_then(|row| row["idx"].as_i64())
        .unwrap_or(0);

    let mut structure = Vec::new();
    let mut blocks = Vec::new();
    for row in list.iter().rev() {
        if row["idx"].as_i64().unwrap_or(0) < from {
            continue;
        }
        let turn_id = match row["turn_id"].as_i64() {
            Some(turn_id) => turn_id,
            None => continue,
        };
        let ranges = reader.turn_ranges(turn_id)?;
        structure.push(json!({"turn": turn_id, "ranges": ranges}));
        let borrowed = reader
            .turn(turn_id)?
            .map(|turn| turn["stop_reason"].as_str() == Some("borrowed"))
            .unwrap_or(true);
        if borrowed || blocks.len() >= LATEST {
            continue;
        }
        for block in reader.blocks(&ranges)?.into_iter().rev() {
            if blocks.len() == LATEST {
                break;
            }
            blocks.push(block);
        }
    }
    structure.reverse();
    blocks.reverse();

    let todo = api.ledger().todo(session)?;
    let todo = if todo.is_empty() {
        Value::Null
    } else {
        serde_json::to_value(&todo).map_err(|e| Refuse::Internal(e.to_string()))?
    };
    Ok(json!({
        "structure": structure,
        "blocks": blocks,
        "todo": todo,
    }))
}
