// The compaction machine: plan from storage, summarize, land, one Borrowed-free tail.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context as _, Result, bail};
use rusqlite::Connection;

use crate::server::agent::r#loop::register::Retention;
use crate::server::agent::request::assembly::compact as cut;
use crate::server::agent::request::assembly::history;
use crate::server::agent::request::content::{BlockSink, RetrySink};
use crate::server::agent::request::message::{Context as Wire, Message};
use crate::server::agent::request::retry::RetryNotice;
use crate::server::agent::request::transport::client::Client;
use crate::server::storage::decode::{BLOCK_COLS, block_from_row, parse_blocks};
use crate::server::storage::{BORROWED, NewBlock, Reader, RequestMeta, Submit, Writer};

use super::{Handle, Inner};
use crate::server::agent::r#loop::Live;

/// The user message that opens a compaction turn.
pub(crate) const COMPACT_MESSAGE: &str = "/compact";

/// One compaction, ready to run.
pub(crate) struct CompactionJob {
    pub(crate) session: i64,
    pub(crate) stop: Arc<AtomicBool>,
    pub(crate) instruction: String,
    pub(crate) focus: String,
    pub(crate) ctx: Wire,
    pub(crate) plan: cut::Plan,
    pub(crate) provider: String,
    pub(crate) snapshot: Snapshot,
}

/// What the compaction's own request needs; the Live's transport is not shared.
pub(crate) struct Snapshot {
    pub(crate) base_url: String,
    pub(crate) api_key: String,
    pub(crate) model: String,
    pub(crate) protocol: crate::server::agent::request::protocols::ProtocolId,
    pub(crate) compat: Option<crate::server::agent::request::protocols::Compat>,
    pub(crate) session_id: Option<String>,
    pub(crate) system: String,
    pub(crate) profile: String,
    pub(crate) max_output_tokens: u32,
}

impl Snapshot {
    pub(crate) fn of(live: &Live) -> Snapshot {
        let transport = &live.transport;
        Snapshot {
            base_url: transport.base_url().to_string(),
            api_key: transport.api_key.clone(),
            model: transport.model().to_string(),
            protocol: *transport.protocol(),
            compat: transport
                .compat_json()
                .and_then(|raw| serde_json::from_str(&raw).ok()),
            session_id: transport.session_id.clone(),
            system: live.settings.system.clone(),
            profile: live.settings.profile.clone(),
            max_output_tokens: live.settings.max_output_tokens,
        }
    }
}

pub(crate) fn plan(
    inner: &Inner,
    live: &Live,
    session: i64,
    retention: Option<Retention>,
    focus: Option<String>,
    stop: Arc<AtomicBool>,
) -> Result<CompactionJob> {
    let conn = Reader::open(&inner.writer.path())?;
    let out = gather(inner, &conn, live, session, retention, focus, stop);
    Reader::close(conn);
    out
}

fn gather(
    inner: &Inner,
    conn: &Connection,
    live: &Live,
    session: i64,
    retention: Option<Retention>,
    focus: Option<String>,
    stop: Arc<AtomicBool>,
) -> Result<CompactionJob> {
    let turns = turns(conn, session)?;
    let plan = cut::plan(
        &turns,
        retention.and_then(Retention::cut),
        &inner.cfg.app.server.request.compact,
    );
    if plan.compacted.is_empty() {
        bail!("an empty session needs no compaction");
    }
    let instruction = super::instruction_of(inner, session);
    let provider: String = conn.query_row(
        "SELECT model_provider FROM sessions WHERE id = ?1",
        [session],
        |r| r.get(0),
    )?;
    let stored: Vec<Message> = turns
        .iter()
        .flat_map(|(_, blocks)| blocks.iter())
        .flat_map(|(_, entries)| entries.iter().cloned())
        .collect();
    let ctx = Wire {
        messages: history::to_wire(&stored, &mut history::Rebuild::default()),
        tools: live.roster.clone(),
    };
    Ok(CompactionJob {
        session,
        stop,
        instruction,
        focus: focus.unwrap_or_default(),
        ctx,
        plan,
        provider,
        snapshot: Snapshot::of(live),
    })
}

/// The turn list, blocks rebuilt into messages.
fn turns(conn: &Connection, session: i64) -> Result<Vec<cut::TurnBlocks>> {
    let mut list = conn.prepare(
        "SELECT s.turn_id, t.blocks, t.stop_reason FROM session_turns s
           JOIN turns t ON t.turn_id = s.turn_id
          WHERE s.session_id = ?1 AND s.idx >= 0 ORDER BY s.idx",
    )?;
    let rounds = list
        .query_map(rusqlite::params![session], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut blocks = conn.prepare(&format!(
        "SELECT {BLOCK_COLS} FROM blocks
          WHERE block_id >= ?1 AND block_id <= ?2 ORDER BY block_id"
    ))?;
    let mut out = Vec::new();
    for (turn_id, index, stop) in rounds {
        if stop.as_deref() == Some(BORROWED) {
            continue;
        }
        let mut entries: Vec<(i64, Vec<Message>)> = Vec::new();
        for [lo, hi] in parse_blocks(&index)? {
            for row in blocks.query_map(rusqlite::params![lo, hi], block_from_row)? {
                let block = row?;
                entries.push((block.id, vec![Message::from_block(&block)?]));
            }
        }
        out.push((turn_id, entries));
    }
    Ok(out)
}

/// One summarization round to its end; the context is untouched on any failure.
pub(crate) fn run(writer: &Writer, job: &CompactionJob) -> Result<()> {
    let live = &job.snapshot;
    let mut transport = Client::new(
        &live.base_url,
        &live.api_key,
        &live.model,
        live.protocol,
        live.compat.as_ref(),
    );
    transport.set_session_id(
        live.session_id
            .clone()
            .unwrap_or_else(|| job.session.to_string()),
    );
    let mut block_sink = SilentSink;
    let mut retry_sink = SilentSink;
    let mut ask =
        |req: &crate::server::agent::request::protocols::ProtocolRequest| -> Result<String> {
            let answer =
                transport.complete(req, &mut block_sink, &mut retry_sink, job.stop.as_ref())?;
            Ok(answer.content)
        };
    let compacted = cut::compact(
        &live.model,
        live.max_output_tokens,
        true,
        live.session_id.as_deref(),
        &job.ctx,
        &job.plan.compacted,
        &job.plan.kept_messages,
        &job.instruction,
        &job.focus,
        &live.system,
        &mut ask,
    )?;
    let summary = cut::summary_message(&compacted.summary)
        .to_block(Some(crate::server::platform::time::now_stamp()));
    writer.push(Submit::Compaction {
        session: job.session,
        summary,
        request: Some(Box::new(archive(live, job))),
        kept: job
            .plan
            .kept
            .iter()
            .map(|k| (k.turn_id, k.from_block))
            .collect(),
        profile: live.profile.clone(),
    })?;
    Ok(())
}

fn archive(live: &Snapshot, job: &CompactionJob) -> RequestMeta {
    RequestMeta {
        model: live.model.clone(),
        provider: job.provider.clone(),
        protocol: live.protocol.as_str().to_string(),
        base_url: live.base_url.clone(),
        profile: live.profile.clone(),
        system: live.system.clone(),
        tools_json: serde_json::to_string(&job.ctx.tools)
            .expect("tool serialization is infallible"),
        max_tokens: live.max_output_tokens,
        compat_json: live
            .compat
            .as_ref()
            .map(|c| serde_json::to_string(c).expect("compat serialization is infallible")),
    }
}

struct SilentSink;

impl BlockSink for SilentSink {
    fn block(&mut self, _block: NewBlock, _done: bool) -> bool {
        true
    }
}

impl RetrySink for SilentSink {
    fn retry(&mut self, _notice: &RetryNotice) {}
}
