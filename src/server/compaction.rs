//! Context compaction — the `/compact` pipeline.
//!
//! User-facing contract (settled in design review):
//!
//! - **Manual trigger only.** `/compact [focus]`; no automatic threshold.
//! - **Block-granular cut.** The retained tail is measured in whole
//!   transcript blocks (request+result pairs never split), counted
//!   backwards from the newest block until the `retain_tail` budget is
//!   exhausted. At least one block is always kept — the newest.
//! - **Prefix-replay summary call.** The summarization request reuses
//!   the *live* context prefix (system + tools + compacted region
//!   verbatim) and appends the instruction as a final user turn. The
//!   gateway's prefix cache serves everything but that last turn —
//!   the summary costs output tokens only.
//! - **Shrink check.** If the summary is not smaller than the region it
//!   replaces, the compaction is rejected and the session untouched.
//! - **Persisted fork point.** A single [`Entry::Compaction`] marker
//!   records `first_kept_entry` + the summary; nothing else. Old entries
//!   stay in the store (the tree keeps the pre-compact branch); token
//!   counts are recomputed on demand, never stored.
//!
//! Token arithmetic uses the same cheap `chars/4` estimate as the
//! loop's `max_tokens` shaping — billing truth remains the gateway's
//! reported usage.

use anyhow::{Context as _, bail};

use crate::server::ai::config::CompactConfig;
use crate::server::ai::types::Message;
use crate::server::entry::Entry;

/// The compaction plan, computed from the live context + transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cut {
    /// Index of the first **kept** entry in the entries slice (the fork
    /// point). Everything before it is compacted into the summary.
    pub first_kept: usize,
    /// Indices of the entries being compacted (`0..first_kept` minus
    /// non-protocol entries — computed here so the shrink check and the
    /// replay request agree on the same region).
    pub compacted: std::ops::Range<usize>,
}

/// Walk the transcript backwards in whole blocks and mark the fork point.
///
/// `blocks` is the domain grouping (request+result glued, arrival order:
/// `grouping::chunks` — the compactor must never see a render-order hoist). The
/// newest block always survives even if it alone exceeds the budget —
/// an empty tail would make the summary the *entire* context, which is
/// exactly the "model forgets what it just did" failure we refuse.
pub fn cut_by_budget(entries: &[Entry], blocks: &[(usize, usize)], retain_tail: usize) -> Cut {
    // chars/4 estimate per entry; tokens ≈ chars/4, so budget in tokens
    // converts to a char budget *4. Kept in chars internally to avoid
    // float rounding at the boundary.
    let char_budget = retain_tail.saturating_mul(4);

    let mut kept_chars = 0usize;
    // Walk newest → oldest. The newest block is claimed unconditionally
    // (an empty tail = the summary *is* the context: refused); each
    // earlier block joins only while the budget allows.
    let mut first_kept = None;
    for (start, end) in blocks.iter().rev() {
        let block_chars: usize = entries[*start..*end].iter().map(entry_chars).sum();
        if first_kept.is_some() && kept_chars + block_chars > char_budget {
            break;
        }
        kept_chars += block_chars;
        first_kept = Some(*start);
    }
    let first_kept = first_kept.unwrap_or(entries.len());

    // Drop metadata-only entries (Error/Name/System/Compaction) from the
    // front edge: they carry no protocol weight and would otherwise sit
    // in the replay request as noise.
    let mut first_kept = first_kept;
    while first_kept < entries.len() && !is_protocol(&entries[first_kept]) {
        first_kept += 1;
    }

    Cut {
        first_kept,
        compacted: 0..first_kept,
    }
}

/// Is this entry protocol-visible (does it replay into the context)?
pub(crate) fn is_protocol(e: &Entry) -> bool {
    matches!(
        e,
        Entry::User { .. }
            | Entry::Assistant { .. }
            | Entry::ToolRequest { .. }
            | Entry::ToolResult { .. }
            | Entry::Compaction { .. }
    )
}

/// Cheap size of one entry (chars; tokens ≈ chars/4).
pub(crate) fn entry_chars(e: &Entry) -> usize {
    match e {
        Entry::User { content } => content.len(),
        Entry::Assistant { content, .. } => content.len(),
        Entry::Reasoning { content } => content.len(),
        Entry::ToolRequest { args, intent, .. } => args.len() + intent.len() + 32,
        Entry::ToolResult { result, .. } => result.len(),
        Entry::Compaction { summary, .. } => summary.len(),
        Entry::Error { text } | Entry::System { text, .. } => text.len(),
        // Markers: they cost nothing in the context (the todo list is
        // re-injected as a note, see `entries_to_context`).
        Entry::Name { .. } | Entry::Todo { .. } => 0,
    }
}

// ---------------------------------------------------------------------------
// Instruction template
// ---------------------------------------------------------------------------

/// The default compaction instruction. dsh's eight-section checkpoint
/// shape; `{{focus}}` carries the user's `/compact <focus>` emphasis.
/// Overridable via `config.yaml → compact.instruction_file` (a plain
/// file read at compaction time — edits take effect immediately).
/// The default compaction instruction.
///
/// omp's `compaction-summary.md`, verbatim (plus the untrusted-data guard from
/// its `summarization-system.md`, folded in because we send this as the last
/// **user** turn of the untouched prefix — see the module doc). `{{focus}}`
/// carries the user's `/compact <focus>` emphasis.
pub const DEFAULT_INSTRUCTION: &str = crate::server::prompts::COMPACT_INSTRUCTION;

/// Render the instruction: substitute `{{focus}}` (empty → drop the line).
pub fn render_instruction(template: &str, focus: &str) -> String {
    if focus.trim().is_empty() {
        // Drop the focus line entirely so the template has no dangling marker.
        template
            .lines()
            .filter(|l| !l.contains("{{focus}}"))
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        template.replace("{{focus}}", &format!("\n用户特别强调：{}", focus.trim()))
    }
}

/// Load the instruction (external file when configured, else the default).
pub fn load_instruction(instruction_file: Option<&std::path::Path>) -> String {
    match instruction_file {
        Some(p) => std::fs::read_to_string(p).unwrap_or_else(|_| DEFAULT_INSTRUCTION.into()),
        None => DEFAULT_INSTRUCTION.into(),
    }
}

// ---------------------------------------------------------------------------
// The replay request
// ---------------------------------------------------------------------------

/// Build the summarization request: the live context (system + tools +
/// every protocol message, each exactly once) + the instruction as a
/// final user turn.
///
/// This is dsh's prefix-replay: everything except the appended turn is
/// byte-identical to the last real request, so the gateway's prefix
/// cache answers it without re-billing the history.
pub fn replay_context(ctx: &crate::server::ai::types::Context) -> crate::server::ai::types::Context {
    let mut req = crate::server::ai::types::Context::new();
    // System + tools ride along: they were part of the cached prefix.
    req.tools = ctx.tools.clone();
    // The live context is the finalized transcript's protocol mapping
    // (entries_to_context over ALL entries), so replaying it wholesale
    // already covers the compacted region exactly once. Re-rendering
    // `cut.compacted` from entries here duplicated it — the summarizer
    // paid ~2x the region and saw every turn twice.
    req.messages = ctx.messages.clone();
    // The instruction as the last user turn.
    req.messages.push(Message::User {
        content: String::new(), // filled by the caller (needs the instruction)
    });
    req
}

// ---------------------------------------------------------------------------
// Assembly
// ---------------------------------------------------------------------------

/// One-shot compaction over a finalized transcript. Returns the new
/// protocol context (system + summary turn + kept region) plus the
/// marker to persist.
///
/// `tokens_of` estimates the summary's token cost (chars/4 here; kept
/// injectable for tests).
pub fn compact(
    system: &str,
    entries: &[Entry],
    cfg: &CompactConfig,
    focus: &str,
    summarize: &mut dyn FnMut(&crate::server::ai::types::Context) -> anyhow::Result<String>,
) -> anyhow::Result<Compacted> {
    if entries.is_empty() {
        bail!("空会话无需压缩");
    }

    let blocks = crate::grouping::chunks(entries)
        .into_iter()
        .map(|r| (r.start, r.end))
        .collect::<Vec<_>>();
    let cut = cut_by_budget(entries, &blocks, cfg.retain_tail);
    if cut.compacted.is_empty() {
        bail!("没有可压缩的历史（尾部已覆盖全部条目）");
    }

    // Region sizes for the shrink check (chars; tokens ≈ /4).
    let region_chars: usize = entries[cut.compacted.clone()].iter().map(entry_chars).sum();

    // The replay request: prefix + compacted region + instruction.
    let live = super::turn::entries_to_context(system, entries);
    let mut req = replay_context(&live);
    let instruction = render_instruction(&load_instruction(cfg.instruction_file.as_deref()), focus);
    // Fill the placeholder user turn.
    let last = req.messages.last_mut().expect("placeholder exists");
    *last = Message::User {
        content: instruction,
    };

    // Summarize (the caller owns the client round-trip).
    let summary = summarize(&req).context("总结请求失败（上下文未受影响，可重试）")?;
    if summary.trim().is_empty() {
        bail!("总结模型返回空内容（上下文未受影响，可重试）");
    }

    // Shrink check: a summary that is not smaller than its region is a
    // net loss — reject instead of shipping a worse context.
    let summary_chars = summary.len();
    if summary_chars >= region_chars {
        bail!(
            "shrink 检查未通过：总结 {}/4 ≈ {} tokens，不小于被压缩区 ≈ {} tokens —— 上下文保持原样",
            summary_chars,
            summary_chars / 4,
            region_chars / 4
        );
    }

    let marker = Entry::Compaction {
        first_kept_entry: cut.first_kept,
        summary: summary.clone(),
    };

    // New protocol context: system + summary as an opening user turn +
    // the kept region's protocol messages.
    let mut new_ctx = crate::server::ai::types::Context::new();
    new_ctx.messages.push(Message::System {
        content: system.to_string(),
    });
    new_ctx.messages.push(Message::User {
        content: format!("{}\n\n<summary>\n{summary}\n</summary>", crate::server::prompts::COMPACT_CONTEXT),
    });
    let kept = &entries[cut.first_kept..];
    let mut kept_ctx = super::turn::entries_to_context(system, kept);
    if matches!(kept_ctx.messages.first(), Some(Message::System { .. })) {
        kept_ctx.messages.remove(0);
    }
    new_ctx.messages.extend(kept_ctx.messages);
    new_ctx.tools = live.tools.clone();

    Ok(Compacted {
        ctx: new_ctx,
        marker,
        tokens_before: region_chars / 4,
        tokens_after: {
            let tail: usize = kept.iter().map(entry_chars).sum();
            tail / 4 + summary_chars / 4
        },
    })
}

/// The outcome: new live context + the marker to persist + display stats.
#[derive(Debug, Clone)]
pub struct Compacted {
    pub ctx: crate::server::ai::types::Context,
    pub marker: Entry,
    /// Approx tokens reclaimed (display only; never persisted).
    pub tokens_before: usize,
    pub tokens_after: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(s: &str) -> Entry {
        Entry::User { content: s.into() }
    }

    #[test]
    fn cut_keeps_newest_block_even_over_budget() {
        // 4 user entries = 4 blocks; budget admits only the last one.
        let es: Vec<Entry> = (0..4).map(|_i| user(&"x".repeat(400))).collect();
        let blocks = crate::grouping::chunks(&es)
            .into_iter()
            .map(|r| (r.start, r.end))
            .collect::<Vec<(usize, usize)>>();
        let cut = cut_by_budget(&es, &blocks, 100); // 100 tok = 400 chars
        assert_eq!(cut.first_kept, 3);
        assert_eq!(cut.compacted, 0..3);
    }

    #[test]
    fn cut_grows_backwards_within_budget() {
        let es: Vec<Entry> = (0..4).map(|_i| user(&"x".repeat(100))).collect();
        let blocks = crate::grouping::chunks(&es)
            .into_iter()
            .map(|r| (r.start, r.end))
            .collect::<Vec<(usize, usize)>>();
        // Budget 200 tok = 800 chars → all four blocks (400 chars) fit.
        let cut = cut_by_budget(&es, &blocks, 200);
        assert_eq!(cut.first_kept, 0);
    }

    #[test]
    fn pair_never_splits() {
        // request+result glued: the pair is one block; the cut cannot
        // land between them.
        let es = vec![
            user("setup"),
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "bash".into(),
                args: "{}".into(),
                intent: String::new(),
                text: String::new(),
                first: true,
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "bash".into(),
                ok: true,
                result: "out".into(),
                details: None,
                duration_ms: 0,
            },
            user("newest"),
        ];
        let blocks = crate::grouping::chunks(&es)
            .into_iter()
            .map(|r| (r.start, r.end))
            .collect::<Vec<(usize, usize)>>();
        // Budget admits "newest" (unconditional) plus the 37-char pair,
        // but NOT "setup": with the pair claimed the fork must land at
        // its start (1), never inside it (2).
        let cut = cut_by_budget(&es, &blocks, 11);
        assert_eq!(cut.first_kept, 1);
    }

    #[test]
    fn focus_line_dropped_when_empty() {
        let t = render_instruction(DEFAULT_INSTRUCTION, "");
        assert!(!t.contains("{{focus}}"));
        assert!(!t.contains("用户特别强调"));
        let f = render_instruction(DEFAULT_INSTRUCTION, "保留大纲");
        assert!(f.contains("保留大纲"));
    }

    #[test]
    fn shrink_rejects_fat_summary() {
        let es: Vec<Entry> = (0..3).map(|_i| user(&"y".repeat(200))).collect();
        let cfg = CompactConfig {
            retain_tail: 10,
            ..Default::default()
        };
        let fat = "z".repeat(2000); // bigger than the region
        let mut summarize =
            |_: &crate::server::ai::types::Context| -> anyhow::Result<String> { Ok(fat.clone()) };
        let r = compact("sys", &es, &cfg, "", &mut summarize);
        assert!(r.is_err());
        assert!(r.err().unwrap().to_string().contains("shrink"));
    }

    #[test]
    fn happy_path_shape() {
        let es: Vec<Entry> = (0..3).map(|_i| user(&"y".repeat(200))).collect();
        let cfg = CompactConfig {
            retain_tail: 10,
            ..Default::default()
        };
        let mut summarize = |_: &crate::server::ai::types::Context| -> anyhow::Result<String> {
            Ok("## 当前工作\n\n无".into())
        };
        let out = compact("sys", &es, &cfg, "", &mut summarize).unwrap();
        // marker lands at the fork
        assert_eq!(
            out.marker,
            Entry::Compaction {
                first_kept_entry: 2,
                summary: "## 当前工作\n\n无".into(),
            }
        );
        // new context: system + summary turn + kept region (no dup system)
        assert_eq!(out.ctx.messages.len(), 3);
        assert!(matches!(out.ctx.messages[0], Message::System { .. }));
        assert!(out.tokens_after < out.tokens_before);
    }

    /// The summarizer must see the transcript exactly once: system + every
    /// protocol message + the instruction. A duplicate replay of the
    /// compacted region (the old `replay_context`) billed the region twice.
    #[test]
    fn summarize_request_never_duplicates_the_region() {
        use crate::server::ai::types::{Context, Message};
        use std::cell::RefCell;
        let es: Vec<Entry> = (0..3)
            .map(|_i| Entry::User {
                content: "y".repeat(200),
            })
            .collect();
        let cfg = CompactConfig {
            retain_tail: 10,
            ..Default::default()
        };
        let shape: RefCell<Vec<usize>> = RefCell::new(Vec::new());
        let mut summarize = |req: &Context| -> anyhow::Result<String> {
            let users = req
                .messages
                .iter()
                .filter(|m| matches!(m, Message::User { content } if content.starts_with('y')))
                .count();
            *shape.borrow_mut() = vec![req.messages.len(), users];
            Ok("## 当前工作\n\n无".into())
        };
        let _ = compact("sys", &es, &cfg, "", &mut summarize).unwrap();
        // system + 3 user turns + instruction; the region (2 turns) appears once.
        assert_eq!(*shape.borrow(), vec![5, 3]);
    }
}
