//! request — everything about sending a network request: `assembly`, `inbound`, `transport`. The
//! outward surface is `start` / `hand_results` / `rebuild`.

pub mod assembly;
pub mod content;
pub mod inbound;
pub mod message;
pub mod transport;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};

use crate::server::agent::request::content::{Answer, AnswerSink, RequestContent};
use crate::server::agent::request::message::{
    AssistantMessage, DEFAULT_EMPTY_REPLY, INTERRUPTED_REPLY, Message, StopReason,
};
use crate::server::agent::request::transport::client::Client;
use crate::server::agent::tools::Spill;
use crate::server::storage::decode::BLOCK_COLS;
use crate::server::storage::{self, NewBlock, RequestMeta, Submit, Writer};

/// One tool call that ran: whatever the tool produced, this holds.
pub struct ToolRun {
    pub call_id: String,
    pub ok: bool,
    pub result: String,
    pub memo: Option<serde_json::Value>,
    pub spill: Option<Spill>,
    pub duration_ms: u64,
}

/// Send one request. Only the session id comes in: the user's message was filed first, its prefix and
/// suffix frozen in the block. `w` writes, `r` reads.
pub fn start(
    w: &Writer,
    r: &Connection,
    transport: &Client,
    content: &RequestContent,
    sink: &mut dyn AnswerSink,
    cancel: &std::sync::atomic::AtomicBool,
) -> Result<Answer> {
    let history = rebuild(r, content.session)?;
    let message = inbound::streamed(
        transport,
        &content.system,
        &history,
        &content.tools,
        content.max_tokens,
        cancel,
        sink,
    )?;
    let stop = message.stop_reason;
    let usage = message.usage.clone();
    let ts = Some(crate::server::platform::time::now_stamp());
    let blocks = new_blocks(&message, ts);
    hand_blocks(w, content.session, blocks)?;
    Ok(Answer {
        stop,
        usage,
        message,
        meta: archive(r, transport, content),
    })
}

/// Read the blocks from the read start and flatten them into messages. The read start is the list's
/// newest compaction turn (or the head of the list).
pub(crate) fn rebuild(r: &Connection, session: i64) -> Result<Vec<Message>> {
    let from: i64 = r.query_row(
        "SELECT COALESCE(MAX(idx), -1) FROM session_turns WHERE session_id = ?1 AND is_compaction",
        [session],
        |r| r.get(0),
    )?;
    let mut list = r.prepare(
        "SELECT t.blocks FROM session_turns s JOIN turns t ON t.turn_id = s.turn_id
          WHERE s.session_id = ?1 AND s.idx >= ?2 ORDER BY s.idx",
    )?;
    let intervals = list
        .query_map(rusqlite::params![session, from], |r| {
            let raw: String = r.get(0)?;
            Ok(crate::server::storage::decode::parse_blocks(&raw).unwrap_or_default())
        })?
        .collect::<rusqlite::Result<Vec<Vec<[i64; 2]>>>>()?;

    let mut blocks = r.prepare(&format!(
        "SELECT {BLOCK_COLS} FROM blocks WHERE block_id BETWEEN ?1 AND ?2 ORDER BY block_id"
    ))?;
    let mut out = Vec::new();
    for turn in intervals {
        for [start, end] in turn {
            let rows = blocks.query_map(
                rusqlite::params![start, end],
                crate::server::storage::decode::block_from_row,
            )?;
            for row in rows {
                out.push(Message::from_block(&row?)?);
            }
        }
    }
    Ok(out)
}

/// What this reply becomes as blocks: with tools, one `tool_request` per call (paired by `call_id`,
/// the text on the first); without them, one `assistant`. A reasoning chain comes first.
fn new_blocks(a: &AssistantMessage, ts: Option<String>) -> Vec<NewBlock> {
    let mut out = Vec::new();
    if let Some(r) = a.reasoning.as_deref().filter(|r| !r.is_empty()) {
        out.push(
            Message::Reasoning {
                content: r.to_string(),
            }
            .to_block(ts.clone()),
        );
    }
    if !a.tool_calls.is_empty() {
        for (i, call) in a.tool_calls.iter().enumerate() {
            out.push(
                Message::ToolRequest {
                    call_id: call.id.clone(),
                    name: call.name().to_string(),
                    args: call.function.arguments.clone(),
                    intent: call.intent(),
                    text: if i == 0 {
                        a.content.clone()
                    } else {
                        String::new()
                    },
                    first: i == 0,
                }
                .to_block(ts.clone()),
            );
        }
        return out;
    }
    let content = if !a.content.is_empty() {
        a.content.clone()
    } else if a.stop_reason == StopReason::Interrupted {
        INTERRUPTED_REPLY.to_string()
    } else {
        DEFAULT_EMPTY_REPLY.to_string()
    };
    out.push(
        Message::Assistant {
            content,
            usage: Some(Message::usage_summary(&a.usage)),
        }
        .to_block(ts),
    );
    out
}

/// Hand the cut blocks to storage. Filing is its business; this only fills the columns.
pub fn hand_blocks(w: &Writer, session: i64, blocks: Vec<NewBlock>) -> Result<()> {
    w.push(Submit::Blocks {
        session,
        blocks,
        // The request side hands in the cut: these blocks are finished, and the
        // writer closes whatever it was still writing for them.
        done: true,
    })?;
    Ok(())
}

/// The blocks for finished tools: one result each, paired by `call_id`. The cut lives only here.
pub fn hand_results(
    w: &Writer,
    session: i64,
    reply: &AssistantMessage,
    ran: &[ToolRun],
) -> Result<()> {
    let ts = Some(crate::server::platform::time::now_stamp());
    let mut blocks = Vec::with_capacity(ran.len());
    for r in ran {
        let name = reply
            .tool_calls
            .iter()
            .find(|c| c.id == r.call_id)
            .map(|c| c.name().to_string())
            .unwrap_or_default();
        let mut block = Message::ToolResult {
            call_id: r.call_id.clone(),
            name,
            ok: r.ok,
            result: r.result.clone(),
            memo: r.memo.clone(),
            duration_ms: r.duration_ms,
        }
        .to_block(ts.clone());
        // A block that spilled carries its full text down: storage lands it as
        // the block's artifact and cites it in the result.
        block.spill = r.spill.as_ref().map(|s| storage::Spill {
            name: s.name.clone(),
            content: s.content.clone(),
        });
        blocks.push(block);
    }
    hand_blocks(w, session, blocks)
}

/// This round's request record: the part that does not change with the conversation, stored as sent.
fn archive(r: &Connection, transport: &Client, content: &RequestContent) -> RequestMeta {
    let provider: Option<String> = r
        .query_row(
            "SELECT model_provider FROM sessions WHERE id = ?1",
            [content.session],
            |r| r.get(0),
        )
        .optional()
        .unwrap_or(None)
        .flatten();
    RequestMeta {
        model: transport.model().to_string(),
        provider: provider.unwrap_or_default(),
        protocol: transport.protocol().to_string(),
        base_url: transport.base_url().to_string(),
        system: content.system.clone(),
        tools_json: serde_json::to_string(&content.tools).expect("tool serialization is infallible"),
        max_tokens: content.max_tokens,
        compat_json: transport.user_compat_json(),
    }
}
