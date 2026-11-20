// `init` — the whole structure, plus the newest blocks.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;

const LATEST: usize = 32;

pub(crate) fn answer(api: &Api, session: i64) -> Result<Value, Refuse> {
    let reader = Reader::open(api.db())?;
    let list = reader.list(session)?;

    let mut structure = Vec::new();
    let mut blocks = Vec::new();
    for row in list.iter().rev() {
        let turn_id = match row["turn_id"].as_i64() {
            Some(turn_id) => turn_id,
            None => continue,
        };
        // A borrowed round holds ranges another round already owns: it takes no place of its own, and its blocks are drawn under the round they were borrowed from.
        let borrowed = reader
            .turn(turn_id)?
            .map(|turn| turn["stop_reason"].as_str() == Some("borrowed"))
            .unwrap_or(true);
        if borrowed {
            continue;
        }
        let ranges = reader.turn_ranges(turn_id)?;
        structure.push(json!({
            "turn": turn_id,
            "ranges": ranges,
            "compact": row["is_compaction"],
        }));
        if blocks.len() >= LATEST {
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
        serde_json::to_value(&todo).map_err(|_| Refuse::new("internal"))?
    };
    // The effort is memory-only: the loop is the only place that knows it, and a fresh front end learns it here rather than from a row.
    let effort = api
        .ledger()
        .effort(session)
        .map(|effort| effort.as_str())
        .unwrap_or("auto");
    Ok(json!({
        "structure": structure,
        "blocks": blocks,
        "todo": todo,
        "effort": effort,
    }))
}
