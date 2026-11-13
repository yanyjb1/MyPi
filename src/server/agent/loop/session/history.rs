//! One session's history: read once, appended as turns land.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::server::agent::request::contract::{Block, History, Kind, Piece, TurnView};
use crate::server::agent::request::message::Message;
use crate::server::storage::Reader;

/// One session's read side. The list is read when the session opens; a turn appends to it as it
/// lands. A rewind or a compaction rewrites the list and asks for a reload.
pub struct Transcript {
    reader: Mutex<Reader>,
    session: i64,
    state: Mutex<State>,
}

struct State {
    messages: Arc<Vec<Message>>,
    /// This session had no history when the transcript was built.
    blank: bool,
}

impl Transcript {
    /// Read the session once and keep it until the session closes or is cut back.
    pub fn open(path: &Path, session: i64) -> Result<Self> {
        let reader = Reader::open(path)?;
        let turns = read_turns(&reader, session)?;
        let blank = turns.is_empty();
        Ok(Self {
            reader: Mutex::new(reader),
            session,
            state: Mutex::new(State {
                messages: Arc::new(flatten(turns)),
                blank,
            }),
        })
    }

    /// The session's messages, shared: a lap never copies them.
    pub fn snapshot(&self) -> Arc<Vec<Message>> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .messages
            .clone()
    }

    /// Did this session have any history before its current turn?
    pub fn blank(&self) -> bool {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).blank
    }

    /// The first turn has begun; no later turn is a first turn.
    pub fn started(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).blank = false;
    }

    /// Append the blocks one lap just landed, in landing order.
    pub fn append(&self, blocks: Vec<Block>) -> Result<()> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let messages = Arc::make_mut(&mut state.messages);
        for block in blocks {
            messages.push(block_message(block)?);
        }
        Ok(())
    }

    /// Append one opening, wake or reminder message the turn wrote.
    pub fn append_user(&self, message: Message) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        Arc::make_mut(&mut state.messages).push(message);
    }

    /// Read the session again: a rewind or a compaction rewrote the list.
    pub fn reload(&self) -> Result<()> {
        let turns = {
            let reader = self.reader.lock().unwrap_or_else(|e| e.into_inner());
            read_turns(&reader, self.session)?
        };
        let blank = turns.is_empty();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.messages = Arc::new(flatten(turns));
        state.blank = blank;
        Ok(())
    }
}

impl History for Transcript {
    fn turns(&self, session: i64) -> Result<Vec<TurnView>> {
        if session != self.session {
            bail!("transcript for {} asked for {session}", self.session);
        }
        let reader = self.reader.lock().unwrap_or_else(|e| e.into_inner());
        read_turns(&reader, self.session)
    }

    fn messages(&self, session: i64) -> Result<Arc<Vec<Message>>> {
        if session != self.session {
            bail!("transcript for {} asked for {session}", self.session);
        }
        Ok(self.snapshot())
    }

    fn holder(&self, reference: &str) -> Result<Option<String>> {
        let reader = self.reader.lock().unwrap_or_else(|e| e.into_inner());
        let rows = reader.artifacts_by_ref(reference)?;
        Ok(rows
            .first()
            .and_then(|row| row["hash"].as_str().map(str::to_string)))
    }
}

/// From the newest compaction marker on, in list order.
fn read_turns(reader: &Reader, session: i64) -> Result<Vec<TurnView>> {
    let list = reader.list(session)?;
    let start = list
        .iter()
        .rposition(|row| row["is_compaction"] == Value::Bool(true))
        .unwrap_or(0);

    let mut ranges = BTreeMap::new();
    for row in reader.turns(session)? {
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
        for block in reader.blocks(&ranges)? {
            pieces.push(piece(&block)?);
        }
        // Only the marked round is the compaction; the ones borrowed after it are plain history.
        let compacted = row["is_compaction"] == Value::Bool(true);
        view.push(TurnView {
            turn,
            pieces,
            compacted,
        });
    }
    Ok(view)
}

/// Every message a turn list carries, in order.
fn flatten(turns: Vec<TurnView>) -> Vec<Message> {
    turns
        .into_iter()
        .flat_map(|turn| turn.sent().into_iter().map(|piece| piece.message))
        .collect()
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

/// One landed block as the request half reads it back.
fn block_message(block: Block) -> Result<Message> {
    Ok(match block.kind {
        Kind::Assistant => Message::Assistant {
            content: block
                .content
                .context("a landed assistant block carries no content")?,
        },
        Kind::Reasoning => Message::Reasoning {
            content: block
                .content
                .context("a landed reasoning block carries no content")?,
        },
        Kind::ToolRequest => Message::Call {
            call_id: block.call_id.context("a landed call carries no id")?,
            name: block.name.context("a landed call carries no name")?,
            args: block.args.context("a landed call carries no arguments")?,
            first: block.first.unwrap_or(false),
        },
        Kind::ToolResult => Message::Result {
            call_id: block.call_id.context("a landed result carries no id")?,
            result: block.result.context("a landed result carries no text")?,
        },
    })
}
