//! Context compaction (`/compact`): a compaction is a **prepared request**. Whole-block cut,
//! prefix-replay summary, and a shrink check.

use anyhow::{Context as _, bail};

use crate::server::agent::request::message::{Message, WireMessage};
use crate::server::agent::request::protocols::ProtocolRequest;
use crate::server::config::CompactConfig;

/// The one construction point for the summary user message: live and rebuilt must be the same bytes.
pub(crate) fn summary_turn(summary: &str) -> WireMessage {
    WireMessage::User {
        content: format!(
            "{}\n\n<summary>\n{summary}\n</summary>",
            crate::server::config::prompts::compact_context()
        )
        .into(),
    }
}

/// One entry of the kept region: which round, from which block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeptTurn {
    pub turn_id: i64,
    pub from_block: Option<i64>,
}

/// The cut: the kept region (which rounds, from where) plus the messages the summary replaces.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// The kept region: one entry per whole round, plus the from-block of the round the cut lands in.
    pub kept: Vec<KeptTurn>,
    /// The messages the summary replaces (from the read start up to the kept region).
    pub compacted: Vec<Message>,
    /// The messages kept verbatim.
    pub kept_messages: Vec<Message>,
}

/// One round in this plan: its id plus its blocks (each block is a run of messages).
pub type TurnBlocks = (i64, Vec<(i64, Vec<Message>)>);

/// Compute the cut. `turns` are the rounds from the read start, each `(round, blocks)`.
pub fn plan(turns: &[TurnBlocks], retention: Option<Retention>, cfg: &CompactConfig) -> Plan {
    enum Mode {
        /// A character budget (tokens ≈ chars/4).
        Chars(usize),
        /// A fixed number of blocks.
        Blocks(usize),
    }
    let mode = match retention {
        Some(Retention::Tokens(n)) => Mode::Chars(n.saturating_mul(4)),
        Some(Retention::Blocks(n)) => Mode::Blocks(n.max(1)),
        None => Mode::Chars(cfg.retain_tail.saturating_mul(4)),
    };
    let mut kept: Vec<KeptTurn> = Vec::new();
    let mut kept_chars = 0usize;
    let mut kept_blocks = 0usize;
    for (turn_id, blocks) in turns.iter().rev() {
        // How many blocks this round keeps from the tail, and from which one.
        let mut from = blocks.len();
        for (j, (_, entries)) in blocks.iter().enumerate().rev() {
            let chars: usize = entries.iter().map(message_chars).sum();
            let fits = match mode {
                // The newest block is always kept (an empty tail is a rejection).
                Mode::Chars(budget) => kept_blocks == 0 || kept_chars + chars <= budget,
                Mode::Blocks(want) => kept_blocks == 0 || kept_blocks < want,
            };
            if !fits {
                break;
            }
            kept_chars += chars;
            kept_blocks += 1;
            from = j;
        }
        if from == blocks.len() {
            break; // 这个回合一块都留不下：保留区到此为止
        }
        kept.push(KeptTurn {
            turn_id: *turn_id,
            from_block: (from > 0).then(|| blocks[from].0),
        });
        match mode {
            Mode::Chars(budget) if kept_chars > budget => break,
            Mode::Blocks(want) if kept_blocks >= want => break,
            _ => {}
        }
    }
    kept.reverse();
    // The boundary falls on a **block**: a whole round goes, or it is kept from some block on.
    let keep_from = |turn_id: i64| -> Option<Option<i64>> {
        kept.iter()
            .find(|k| k.turn_id == turn_id)
            .map(|k| k.from_block)
    };
    let mut compacted = Vec::new();
    let mut kept_messages = Vec::new();
    for (turn_id, blocks) in turns {
        match keep_from(*turn_id) {
            // The whole round is kept.
            Some(None) => {
                for (_, entries) in blocks {
                    kept_messages.extend(entries.iter().cloned());
                }
            }
            // The boundary round: before the cut goes to the summary, after it stays.
            Some(Some(from)) => {
                for (block_id, entries) in blocks {
                    let dest = if *block_id < from {
                        &mut compacted
                    } else {
                        &mut kept_messages
                    };
                    dest.extend(entries.iter().cloned());
                }
            }
            // The whole round goes to the summary.
            None => {
                for (_, entries) in blocks {
                    compacted.extend(entries.iter().cloned());
                }
            }
        }
    }
    Plan {
        kept,
        compacted,
        kept_messages,
    }
}

/// Cheap size of one message (chars; tokens ≈ chars/4).
pub fn message_chars(e: &Message) -> usize {
    match e {
        Message::User { content, .. } => content.len(),
        Message::Assistant { content, .. } => content.len(),
        Message::Reasoning { content } => content.len(),
        Message::ToolRequest { args, intent, .. } => args.len() + intent.len() + 32,
        Message::ToolResult { result, .. } => result.len(),
    }
}

// ---- instruction template ----

/// The default instruction: omp's `compaction-summary.md` plus the untrusted-data guard. `{{focus}}`
/// carries the emphasis.
pub fn render_instruction(template: &str, focus: &str) -> String {
    if focus.trim().is_empty() {
        // Drop the focus line entirely so the template has no dangling marker.
        template
            .lines()
            .filter(|l| !l.contains("{{focus}}"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        template.replace(
            "{{focus}}",
            &format!(
                "\nThe user specifically asks you to focus on: {}",
                focus.trim()
            ),
        )
    }
}

/// How much of the transcript tail to keep verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    /// A token budget (the cheap `chars/4` estimate, same as `max_tokens`
    /// shaping). Whole blocks are claimed newest-first while they fit.
    Tokens(usize),
    /// A fixed number of newest blocks, whatever they weigh.
    Blocks(usize),
}

// ---- the replay request ----

/// Build the summary request: the live context (tools + every protocol message once) + the instruction
/// last. Except that last turn, byte-identical to the previous real request.
pub fn replay_context(
    ctx: &crate::server::agent::request::message::Context,
    system: &str,
) -> crate::server::agent::request::message::Context {
    let mut req = crate::server::agent::request::message::Context::new();
    // Tools ride along: they were part of the cached prefix.
    req.tools = ctx.tools.clone();
    // Replaying the whole context is enough: re-rendering by entry would count the compacted region twice.
    req.messages.extend(ctx.messages.iter().cloned());
    // The instruction as the last user turn.
    req.messages.push(WireMessage::User {
        content: String::new().into(), // filled by the caller (needs the instruction)
    });
    req.messages = super::with_system(req.messages, system);
    req
}

// ---- assembly ----

/// Run one compaction: summarize `compacted` into `summary`. A summary no smaller than the region it
/// replaces is rejected.
pub fn compact(
    model: &str,
    max_output_tokens: u32,
    stream: bool,
    session_id: Option<&str>,
    ctx: &crate::server::agent::request::message::Context,
    compacted: &[Message],
    kept: &[Message],
    instruction: &str,
    focus: &str,
    system: &str,
    summarize: &mut dyn FnMut(&ProtocolRequest) -> anyhow::Result<String>,
) -> anyhow::Result<Compacted> {
    if compacted.is_empty() {
        bail!("an empty session needs no compaction");
    }

    // Region sizes for the shrink check (chars; tokens ≈ /4).
    let region_chars: usize = compacted.iter().map(message_chars).sum();

    // The replay request: the live context + the instruction as its last turn.
    let mut replay = replay_context(ctx, system);
    let instruction = render_instruction(instruction, focus);
    // Fill the placeholder user turn.
    let last = replay.messages.last_mut().expect("placeholder exists");
    *last = WireMessage::User {
        content: instruction.into(),
    };
    let req = ProtocolRequest {
        model: model.to_string(),
        max_output_tokens,
        stream,
        messages: replay.messages,
        tools: replay.tools,
        session_id: session_id.map(str::to_string),
    };

    // Summarize (the caller owns the client round-trip).
    let summary = summarize(&req)
        .context("the summarization request failed (the context is untouched; safe to retry)")?;
    if summary.trim().is_empty() {
        bail!("the summarizing model returned nothing (the context is untouched; safe to retry)");
    }

    // Shrink check: a summary that is not smaller than its region is a
    // net loss — reject instead of shipping a worse context.
    let summary_chars = summary.len();
    if summary_chars >= region_chars {
        bail!(
            "shrink check failed: the summary is {}/4 ≈ {} tokens, not smaller than the region it replaces ≈ {} tokens — the context is untouched",
            summary_chars,
            summary_chars / 4,
            region_chars / 4
        );
    }

    Ok(Compacted {
        summary,
        tokens_before: region_chars / 4,
        tokens_after: {
            let tail: usize = kept.iter().map(message_chars).sum();
            tail / 4 + summary_chars / 4
        },
    })
}

/// The compaction round's own block: the summary message, read back as an ordinary user message.
pub fn summary_message(summary: &str) -> Message {
    let WireMessage::User { content } = summary_turn(summary) else {
        unreachable!("summary_turn 造的就是一条 user 消息")
    };
    Message::User {
        content: content.text().to_string(),
        prefix: None,
        suffix: None,
    }
}

/// The result: the summary text plus two display-only estimates (neither is stored).
#[derive(Debug, Clone)]
pub struct Compacted {
    pub summary: String,
    /// The estimated size of the replaced region (display).
    pub tokens_before: usize,
    /// The estimated size of the summary plus the kept region (display).
    pub tokens_after: usize,
}
