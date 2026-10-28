//! The compaction machinery: gather, plan, run, and settle the round.

use anyhow::Result;
use rusqlite::Connection;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::server::agent::r#loop::Live;
use crate::server::agent::request::assembly::compact::{self, Plan, TurnBlocks};
use crate::server::agent::request::content::{BlockSink, RetrySink};
use crate::server::agent::request::message::{Context, Message};
use crate::server::agent::request::protocols::ProtocolRequest;
use crate::server::agent::request::retry::RetryNotice;
use crate::server::storage::decode::{BLOCK_COLS, block_from_row, parse_blocks};
use crate::server::storage::{BORROWED, NewBlock, RequestMeta, Submit, Writer};

use super::relay::Relay;
use super::running::Running;
use super::sessions::Sessions;

/// One compaction, ready to run.
pub(crate) struct Job {
    pub(crate) session: i64,
    pub(crate) turn: i64,
    pub(crate) live: Live,
    pub(crate) stop: Arc<AtomicBool>,
    pub(crate) instruction: String,
    pub(crate) focus: String,
    pub(crate) ctx: Context,
    pub(crate) plan: Plan,
    pub(crate) provider: String,
}

pub(crate) fn turns(conn: &Connection, session: i64, from: i64) -> Result<Vec<TurnBlocks>> {
    let mut list = conn.prepare(
        "SELECT s.turn_id, t.blocks, t.stop_reason FROM session_turns s
           JOIN turns t ON t.turn_id = s.turn_id
          WHERE s.session_id = ?1 AND s.idx >= ?2 ORDER BY s.idx",
    )?;
    let rounds = list
        .query_map(rusqlite::params![session, from], |r| {
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

pub(crate) fn run(writer: Writer, relay: Relay, sessions: Sessions, running: Running, job: Job) {
    match summarize(&writer, &job) {
        Ok(()) => sessions.arm_profile(job.session),
        Err(e) => {
            crate::server::platform::log::warn(
                "api",
                format!(
                    "session {}: the compaction did not land (the context is untouched): {e:#}",
                    job.session
                ),
            );
            if let Err(e) = writer.push(Submit::RoundEnd {
                session: job.session,
                stop_reason: None,
                usage: None,
            }) {
                crate::server::platform::log::warn(
                    "api",
                    format!(
                        "session {}: the dead compaction did not settle: {e:#}",
                        job.session
                    ),
                );
            }
        }
    }
    relay.ended(job.session, job.turn, None, None);
    running.release(job.session);
    if relay.watched(job.session) {
        sessions.put(job.session, job.live);
    } else {
        sessions.drop_live(job.session);
    }
}

fn summarize(writer: &Writer, job: &Job) -> Result<()> {
    let live = &job.live;
    let mut sink = Silent;
    let mut retry = Silent;
    let mut ask = |req: &ProtocolRequest| -> Result<String> {
        let answer = live
            .transport
            .complete(req, &mut sink, &mut retry, job.stop.as_ref())?;
        Ok(answer.content)
    };
    let compacted = compact::compact(
        live.transport.model(),
        live.settings.max_output_tokens,
        true,
        live.transport.session_id.as_deref(),
        &job.ctx,
        &job.plan.compacted,
        &job.plan.kept_messages,
        &job.instruction,
        &job.focus,
        &live.settings.system,
        &mut ask,
    )?;
    let summary = compact::summary_message(&compacted.summary)
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
        profile: live.settings.profile.clone(),
    })?;
    Ok(())
}

fn archive(live: &Live, job: &Job) -> RequestMeta {
    RequestMeta {
        model: live.transport.model().to_string(),
        provider: job.provider.clone(),
        protocol: live.transport.protocol().as_str().to_string(),
        base_url: live.transport.base_url().to_string(),
        profile: live.settings.profile.clone(),
        system: live.settings.system.clone(),
        tools_json: serde_json::to_string(&live.roster).expect("tool serialization is infallible"),
        max_tokens: live.settings.max_output_tokens,
        compat_json: live.transport.compat_json(),
    }
}

struct Silent;

impl BlockSink for Silent {
    fn block(&mut self, _block: NewBlock, _done: bool) -> bool {
        true
    }
}

impl RetrySink for Silent {
    fn retry(&mut self, _notice: &RetryNotice) {}
}
