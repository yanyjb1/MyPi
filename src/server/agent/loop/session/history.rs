//! The session's stored history, read on demand through storage's reader.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::server::agent::request::contract::{History, Piece, TurnView};
use crate::server::agent::request::message::Message;
use crate::server::storage::Reader;

/// One session's rows, decoded into what the request half reads.
pub struct Transcript {
    reader: Reader,
}

impl Transcript {
    pub fn open(path: &Path) -> Result<Self> {
        Ok(Self {
            reader: Reader::open(path)?,
        })
    }
}

impl History for Transcript {
    fn turns(&self, session: i64) -> Result<Vec<TurnView>> {
        let list = self.reader.list(session)?;
        let start = list
            .iter()
            .rposition(|row| row["is_compaction"] == Value::Bool(true))
            .unwrap_or(0);

        let mut ranges = BTreeMap::new();
        for row in self.reader.turns(session)? {
            let Some(turn) = row["turn_id"].as_i64() else {
                continue;
            };
            ranges.insert(turn, intervals(&row["ranges"])?);
        }

        let mut view = Vec::new();
        for row in list.iter().skip(start) {
            let Some(turn) = row["turn_id"].as_i64() else {
                continue;
            };
            let Some(ranges) = ranges.remove(&turn) else {
                continue;
            };
            let mut pieces = Vec::new();
            for block in self.reader.blocks(&ranges)? {
                pieces.push(piece(&block)?);
            }
            view.push(TurnView { ranges, pieces });
        }
        Ok(view)
    }

    fn holder(&self, reference: &str) -> Result<Option<String>> {
        let rows = self.reader.artifacts_by_ref(reference)?;
        Ok(rows
            .first()
            .and_then(|row| row["hash"].as_str().map(str::to_string)))
    }
}

fn intervals(raw: &Value) -> Result<Vec<[i64; 2]>> {
    let Some(items) = raw.as_array() else {
        bail!("a turn carries no ranges");
    };
    items
        .iter()
        .map(|item| {
            let pair = item
                .as_array()
                .filter(|pair| pair.len() == 2)
                .context("a turn's range is not a pair")?;
            Ok([pair[0].as_i64().unwrap_or(0), pair[1].as_i64().unwrap_or(0)])
        })
        .collect()
}

fn piece(row: &Value) -> Result<Piece> {
    Ok(Piece {
        block: row["id"].as_i64().context("a block row carries no id")?,
        message: message(row)?,
    })
}

/// One stored block as the request half reads it. A field its kind needs but does not have is an error.
fn message(row: &Value) -> Result<Message> {
    let text = |key: &str| row[key].as_str().map(str::to_string);
    let need = |key: &str| {
        text(key).with_context(|| {
            format!(
                "a {} block carries no {key}",
                row["kind"].as_str().unwrap_or("?")
            )
        })
    };
    Ok(match row["kind"].as_str().unwrap_or("") {
        "user" | "wake" => Message::User {
            prefix: text("prefix"),
            content: need("content")?,
            suffix: text("suffix"),
        },
        "assistant" => Message::Assistant {
            content: need("content")?,
        },
        "reasoning" => Message::Reasoning {
            content: need("content")?,
        },
        "tool_request" => Message::Call {
            call_id: need("call_id")?,
            name: need("name")?,
            args: need("args")?,
            first: row["first"].as_bool().unwrap_or(false),
        },
        "tool_result" => Message::Result {
            call_id: need("call_id")?,
            result: need("result")?,
        },
        other => bail!("a block carries an unknown kind: {other}"),
    })
}
