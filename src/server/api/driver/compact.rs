//! The compaction door's machinery: gather the transcript, plan the cut, run the summarization
//! round, hand the result to storage. The round trip runs on its own thread.

use anyhow::{Context as _, Result};
use rusqlite::Connection;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::server::agent::r#loop::Live;
use crate::server::agent::request::assembly::body::body_json;
use crate::server::agent::request::assembly::compact::{self, Plan, TurnBlocks};
use crate::server::agent::request::content::AnswerSink;
use crate::server::agent::request::inbound::decode;
use crate::server::agent::request::message::{Context, Message};
use crate::server::agent::request::transport;
use crate::server::storage::decode::{BLOCK_COLS, block_from_row, parse_blocks};
use crate::server::storage::{BORROWED, RequestMeta, Submit, Writer};

use super::relay::Relay;
use super::running::Running;
use super::sessions::Sessions;

/// One compaction, ready to run.
pub(crate) struct Job {
    pub(crate) session: i64,
    /// The session's state, borrowed for the summarization.
    pub(crate) live: Live,
    /// The round's stop flag (the door claimed the session).
    pub(crate) stop: Arc<AtomicBool>,
    /// The instruction template, and the user's focus line (empty = none).
    pub(crate) instruction: String,
    pub(crate) focus: String,
    /// The context the summarization request replays, and the cut it lands.
    pub(crate) ctx: Context,
    pub(crate) plan: Plan,
    /// The provider this session is on; the archive records it.
    pub(crate) provider: String,
}

/// The session's rounds from `from` on, each with its blocks as messages — the shape
/// [`compact::plan`] reads. A borrowed round is skipped.
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

/// Run one compaction, then give the session back.
pub(crate) fn run(
    writer: Writer,
    relay: Relay,
    sessions: Sessions,
    running: Running,
    job: Job,
) {
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
            // Nothing reached storage, so nothing there will end this round: settle it here.
            if let Err(e) = writer.push(Submit::RoundEnd {
                session: job.session,
                stop_reason: None,
                usage: None,
            }) {
                crate::server::platform::log::warn(
                    "api",
                    format!("session {}: the dead compaction did not settle: {e:#}", job.session),
                );
            }
        }
    }
    // Nothing runs in this session any more: the state goes back when somebody watches it.
    running.release(job.session);
    if relay.watched(job.session) {
        sessions.put(job.session, job.live);
    } else {
        sessions.drop_live(job.session);
    }
}

/// Summarize the planned region and land the compaction round.
fn summarize(writer: &Writer, job: &Job) -> Result<()> {
    let live = &job.live;
    let transport = &live.transport;
    let mut silent = Silent;
    let mut ask = |req: &Context| -> Result<String> {
        let messages = serde_json::to_value(&req.messages)
            .context("the replay context does not serialize")?;
        let tools =
            serde_json::to_value(&req.tools).context("the tool manuals do not serialize")?;
        let body = body_json(
            &transport.model,
            live.settings.max_tokens,
            true,
            &messages,
            &tools,
            &transport.compat,
            transport.session_id.as_deref(),
        );
        let reader = transport::send(transport, &body)?;
        let answer = decode::streamed(reader, transport, job.stop.as_ref(), &mut silent)?;
        Ok(answer.content)
    };
    let compacted = compact::compact(
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
    })?;
    Ok(())
}

/// The summarization request's archive: what does not change with the
/// conversation.
fn archive(live: &Live, job: &Job) -> RequestMeta {
    RequestMeta {
        model: live.transport.model().to_string(),
        provider: job.provider.clone(),
        protocol: live.transport.protocol().to_string(),
        base_url: live.transport.base_url().to_string(),
        system: live.settings.system.clone(),
        tools_json: serde_json::to_string(&live.roster)
            .expect("tool serialization is infallible"),
        max_tokens: live.settings.max_tokens,
        compat_json: live.transport.user_compat_json(),
    }
}

/// The summarization's own output is not streamed anywhere: the summary lands
/// as a block when it is complete.
struct Silent;

impl AnswerSink for Silent {
    fn content(&mut self, _text: &str) -> bool {
        true
    }

    fn reasoning(&mut self, _text: &str) -> bool {
        true
    }
}
