pub mod assembly;
pub mod content;
pub mod message;
pub mod protocols;
pub mod retry;
pub mod transport;

use std::sync::atomic::AtomicBool;

use anyhow::{Result, bail};
use rusqlite::{Connection, OptionalExtension};

use crate::server::agent::request::content::{Answer, BlockSink, RequestContent, RetrySink};
use crate::server::agent::request::message::AssistantMessage;
use crate::server::agent::request::transport::client::Client;
use crate::server::agent::tools::Spill;
use crate::server::storage::decode::BLOCK_COLS;
use crate::server::storage::{self, Effect, NewBlock, RequestMeta, Submit, Writer};

pub struct ToolRun {
    pub call_id: String,
    pub ok: bool,
    pub result: String,
    pub memo: Option<serde_json::Value>,
    pub spill: Option<Spill>,
    pub duration_ms: u64,
    pub effects: Vec<Effect>,
    pub stop: bool,
}

pub fn start(
    w: &Writer,
    r: &Connection,
    client: &Client,
    content: &RequestContent,
    retry: &mut dyn RetrySink,
    cancel: &AtomicBool,
) -> Result<Answer> {
    let history = rebuild(r, content.session)?;
    let req = assembly::build(
        &client.model,
        content.max_output_tokens,
        true,
        &content.system,
        &history,
        &content.tools,
        client.session_id.as_deref(),
    );
    let mut sink = WriterSink {
        w,
        session: content.session,
        ts: Some(crate::server::platform::time::now_stamp()),
        error: None,
    };
    let message = client.complete(&req, &mut sink, retry, cancel)?;
    if let Some(e) = sink.error {
        return Err(e);
    }
    Ok(Answer {
        stop: message.stop_reason,
        usage: message.usage.clone(),
        message,
        meta: archive(r, client, content),
    })
}

pub(crate) fn rebuild(r: &Connection, session: i64) -> Result<Vec<message::Message>> {
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
                out.push(message::Message::from_block(&row?)?);
            }
        }
    }
    Ok(out)
}

pub fn hand_results(
    w: &Writer,
    session: i64,
    reply: &AssistantMessage,
    ran: &[ToolRun],
) -> Result<()> {
    for call in &reply.tool_calls {
        if !ran.iter().any(|r| r.call_id == call.id) {
            bail!("tool call {} has no result", call.id);
        }
    }
    let ts = Some(crate::server::platform::time::now_stamp());
    let mut blocks = Vec::with_capacity(ran.len());
    let mut effects = Vec::new();
    for r in ran {
        let name = reply
            .tool_calls
            .iter()
            .find(|c| c.id == r.call_id)
            .map(|c| c.name().to_string())
            .unwrap_or_default();
        let mut block = message::Message::ToolResult {
            call_id: r.call_id.clone(),
            name,
            ok: r.ok,
            result: r.result.clone(),
            memo: r.memo.clone(),
            duration_ms: r.duration_ms,
        }
        .to_block(ts.clone());
        block.spill = r.spill.as_ref().map(|s| storage::Spill {
            name: s.name.clone(),
            content: s.content.clone(),
        });
        blocks.push(block);
        effects.extend(r.effects.iter().cloned());
    }
    w.push(Submit::Blocks {
        session,
        blocks,
        done: true,
        effects,
    })?;
    Ok(())
}

fn archive(r: &Connection, client: &Client, content: &RequestContent) -> RequestMeta {
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
        model: client.model().to_string(),
        provider: provider.unwrap_or_default(),
        protocol: client.protocol().as_str().to_string(),
        base_url: client.base_url().to_string(),
        profile: content.profile.clone(),
        system: content.system.clone(),
        tools_json: serde_json::to_string(&content.tools)
            .expect("tool serialization is infallible"),
        max_tokens: content.max_output_tokens,
        compat_json: client.compat_json(),
    }
}

struct WriterSink<'a> {
    w: &'a Writer,
    session: i64,
    ts: Option<String>,
    error: Option<anyhow::Error>,
}

impl BlockSink for WriterSink<'_> {
    fn block(&mut self, mut block: NewBlock, done: bool) -> bool {
        if self.error.is_some() {
            return false;
        }
        block.ts = self.ts.clone();
        match self.w.push(Submit::Blocks {
            session: self.session,
            blocks: vec![block],
            done,
            effects: Vec::new(),
        }) {
            Ok(_) => true,
            Err(e) => {
                self.error = Some(e);
                false
            }
        }
    }
}
