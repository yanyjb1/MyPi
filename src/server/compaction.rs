//! Context compaction — the `/compact` pipeline.
//!
//! User-facing contract (settled in design review):
//!
//! - **Manual trigger only.** `/compact [<n>[k]] [focus]`; no automatic
//!   threshold. The optional leading token picks how much tail to keep:
//!   `18k` = an 18k-token budget, `18` = the newest 18 blocks, absent = the
//!   config's `compact.retain_tail`. Anything after it is the focus.
//! - **Block-granular cut.** The retained tail is measured in whole
//!   transcript blocks (request+result pairs never split), counted
//!   backwards from the newest block until the budget is exhausted —
//!   or, in block-count mode, until that many blocks are claimed. At
//!   least one block is always kept — the newest.
//! - **Prefix-replay summary call.** The summarization request reuses
//!   the *live* context prefix (system + tools + compacted region
//!   verbatim) and appends the instruction as a final user turn. The
//!   gateway's prefix cache serves everything but that last turn —
//!   the summary costs output tokens only.
//! - **Instruction look-up.** The active profile's own instruction first
//!   (its `compact.md` re-read every time, or the string the binary ships
//!   for a built-in profile), then `config.yaml → compact.instruction_file`,
//!   then the built-in. A named-but-unreadable file is an error, not a
//!   silent fall back.
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

use anyhow::{Context as _, anyhow, bail};

use crate::server::ai::config::CompactConfig;
use crate::server::ai::types::Message;
use crate::server::entry::Entry;
use crate::server::profile::CompactSource;

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
    let first_kept = drop_front_metadata(entries, first_kept.unwrap_or(entries.len()));

    Cut {
        first_kept,
        compacted: 0..first_kept,
    }
}

/// Keep the newest `keep` whole blocks, regardless of size.
///
/// The block-count sibling of [`cut_by_budget`]: same block granularity, same
/// "the newest block always survives", same front-edge trim. Use it when the
/// transcript's blocks are uneven enough that a token budget decides
/// unpredictably.
pub fn cut_by_blocks(entries: &[Entry], blocks: &[(usize, usize)], keep: usize) -> Cut {
    // `keep == 0` would leave the summary as the entire context — the same
    // failure `cut_by_budget` refuses by always claiming the newest block.
    let keep = keep.max(1);
    let take = blocks.len().min(keep);
    let first_kept = match blocks.len().checked_sub(take) {
        Some(i) => blocks[i].0,
        None => entries.len(),
    };
    let first_kept = drop_front_metadata(entries, first_kept);
    Cut {
        first_kept,
        compacted: 0..first_kept,
    }
}

/// Drop metadata-only entries (Error/Name/System/Compaction) from the front
/// edge of the kept region: they carry no protocol weight and would otherwise
/// sit in the replay request as noise.
fn drop_front_metadata(entries: &[Entry], mut first_kept: usize) -> usize {
    while first_kept < entries.len() && !is_protocol(&entries[first_kept]) {
        first_kept += 1;
    }
    first_kept
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

/// The default compaction instruction: omp's `compaction-summary.md`, verbatim
/// (plus the untrusted-data guard from its `summarization-system.md`, folded in
/// because we send this as the last **user** turn of the untouched prefix — see
/// the module doc). `{{focus}}` carries the user's emphasis.
///
/// Overridable twice: the active profile's `compact.md`, then
/// `config.yaml → compact.instruction_file`. Both are plain files read at
/// compaction time, so edits take effect immediately.
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
        template.replace("{{focus}}", &format!("\nThe user specifically asks you to focus on: {}", focus.trim()))
    }
}

/// Load the compaction instruction. The look-up order is the profile's own
/// (a `compact.md` re-read every time, or a string shipped in the binary),
/// then `config.yaml → compact.instruction_file`, then the built-in default.
///
/// The profile side is **fallible on purpose**: naming a file that cannot be
/// read is a mistake the user must see, not a silent slide back to the
/// built-in prompt. The config file keeps its historical degrade (a missing
/// `instruction_file` falls back), because that path is the older, documented
/// one and callers rely on it.
pub fn load_instruction(
    profile: Option<&CompactSource>,
    instruction_file: Option<&std::path::Path>,
) -> anyhow::Result<String> {
    if let Some(src) = profile {
        return match src {
            CompactSource::File(p) => std::fs::read_to_string(p).with_context(|| {
                format!("failed to read the profile compact.md: {}", p.display())
            }),
            CompactSource::Builtin(text) => Ok((*text).into()),
        };
    }
    Ok(match instruction_file {
        Some(p) => std::fs::read_to_string(p).unwrap_or_else(|_| DEFAULT_INSTRUCTION.into()),
        None => DEFAULT_INSTRUCTION.into(),
    })
}

/// How much of the transcript tail to keep verbatim.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    /// A token budget (the cheap `chars/4` estimate, same as `max_tokens`
    /// shaping). Whole blocks are claimed newest-first while they fit.
    Tokens(usize),
    /// A fixed number of newest blocks, whatever they weigh. The escape hatch
    /// for a transcript whose blocks are unusually fat or thin, where a token
    /// budget keeps an unpredictable amount.
    Blocks(usize),
}

/// Parse the `/compact` argument into (retention, focus).
///
/// Grammar — the leading token decides, the rest is the focus:
///
/// ```text
/// /compact                 → (None, "")             config's retain_tail
/// /compact 20k             → (Tokens(20000), "")    k is the only unit
/// /compact 18              → (Blocks(18), "")       bare number = blocks
/// /compact 18 关注认证      → (Blocks(18), "关注认证")
/// /compact 关注认证         → (None, "关注认证")
/// ```
///
/// `None` means "the config's `retain_tail`", not "keep nothing".
///
/// A number that does not fit its type is **refused**, not clamped: silently
/// compacting with a different budget than the one typed is the kind of
/// surprise this command exists to avoid.
pub fn parse_retention(args: &str) -> anyhow::Result<(Option<Retention>, String)> {
    let trimmed = args.trim();
    if trimmed.is_empty() {
        return Ok((None, String::new()));
    }
    let (head, rest) = match trimmed.split_once(char::is_whitespace) {
        Some((h, r)) => (h, r.trim()),
        None => (trimmed, ""),
    };
    let retention = if let Some(digits) = head.strip_suffix('k').or_else(|| head.strip_suffix('K')) {
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Ok((None, trimmed.to_string()));
        }
        let n: usize = digits
            .parse()
            .map_err(|_| anyhow!("`{head}` is too large — the token budget overflows"))?;
        Some(Retention::Tokens(n.saturating_mul(1000)))
    } else if !head.is_empty() && head.bytes().all(|b| b.is_ascii_digit()) {
        let n: usize = head
            .parse()
            .map_err(|_| anyhow!("`{head}` is too large — the block count overflows"))?;
        if n == 0 {
            bail!("the block count must be at least 1 (keeping 0 blocks leaves no context)");
        }
        Some(Retention::Blocks(n))
    } else {
        None
    };
    match retention {
        Some(r) => Ok((Some(r), rest.to_string())),
        // The whole line is prose: all of it is the focus.
        None => Ok((None, trimmed.to_string())),
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
        content: String::new().into(), // filled by the caller (needs the instruction)
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
    profile_instruction: Option<&CompactSource>,
    retention: Option<Retention>,
    focus: &str,
    summarize: &mut dyn FnMut(&crate::server::ai::types::Context) -> anyhow::Result<String>,
) -> anyhow::Result<Compacted> {
    if entries.is_empty() {
        bail!("an empty session needs no compaction");
    }

    let blocks = crate::grouping::chunks(entries)
        .into_iter()
        .map(|r| (r.start, r.end))
        .collect::<Vec<_>>();
    let cut = match retention {
        None => cut_by_budget(entries, &blocks, cfg.retain_tail),
        Some(Retention::Tokens(n)) => cut_by_budget(entries, &blocks, n),
        Some(Retention::Blocks(n)) => cut_by_blocks(entries, &blocks, n),
    };
    if cut.compacted.is_empty() {
        bail!("nothing to compact (the tail already covers every entry)");
    }

    // Region sizes for the shrink check (chars; tokens ≈ /4).
    let region_chars: usize = entries[cut.compacted.clone()].iter().map(entry_chars).sum();

    // The replay request: prefix + compacted region + instruction.
    let live = super::turn::entries_to_context(system, entries);
    let mut req = replay_context(&live);
    let instruction = render_instruction(
        &load_instruction(profile_instruction, cfg.instruction_file.as_deref())?,
        focus,
    );
    // Fill the placeholder user turn.
    let last = req.messages.last_mut().expect("placeholder exists");
    *last = Message::User {
        content: instruction.into(),
    };

    // Summarize (the caller owns the client round-trip).
    let summary = summarize(&req).context("the summarization request failed (the context is untouched; safe to retry)")?;
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
        content: format!("{}\n\n<summary>\n{summary}\n</summary>", crate::server::prompts::COMPACT_CONTEXT)
        .into(),
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

    fn blocks_of(es: &[Entry]) -> Vec<(usize, usize)> {
        crate::grouping::chunks(es)
            .into_iter()
            .map(|r| (r.start, r.end))
            .collect()
    }

    /// 块数是**不管大小**的：同样的条目，token 预算只留得下两块，块数能留三块。
    #[test]
    fn cut_by_blocks_keeps_the_newest_n_regardless_of_size() {
        let es: Vec<Entry> = (0..6).map(|_i| user(&"x".repeat(5_000))).collect();
        let blocks = blocks_of(&es);
        // 5k 字符一块 ≈ 1.2k token；3k token 预算只装得下两块。
        assert_eq!(cut_by_budget(&es, &blocks, 3_000).first_kept, 4);
        // 块数口径下，三块就是三块，不看它们多重。
        assert_eq!(cut_by_blocks(&es, &blocks, 3).first_kept, 3);
        assert_eq!(cut_by_blocks(&es, &blocks, 3).compacted, 0..3);
    }

    /// 比历史还多的块数 = 全部保留，等于没得压。
    #[test]
    fn cut_by_blocks_beyond_the_transcript_keeps_everything() {
        let es: Vec<Entry> = (0..3).map(|_i| user("x")).collect();
        let cut = cut_by_blocks(&es, &blocks_of(&es), 99);
        assert_eq!(cut.compacted, 0..0);
        assert_eq!(cut.first_kept, 0);
    }

    /// 0 块会被夹到 1：保留零个块等于把总结当成全部上下文。
    #[test]
    fn cut_by_blocks_always_keeps_at_least_one() {
        let es: Vec<Entry> = (0..3).map(|_i| user("x")).collect();
        assert_eq!(cut_by_blocks(&es, &blocks_of(&es), 0).first_kept, 2);
    }

    #[test]
    fn retention_argument_parses() {
        // 裸命令 = 用配置里的默认，不是「保留零个」。
        assert_eq!(parse_retention("").unwrap(), (None, String::new()));
        assert_eq!(parse_retention("   ").unwrap(), (None, String::new()));
        // k 是唯一认的单位。
        assert_eq!(
            parse_retention("20k").unwrap(),
            (Some(Retention::Tokens(20_000)), String::new())
        );
        assert_eq!(
            parse_retention("1K").unwrap(),
            (Some(Retention::Tokens(1_000)), String::new())
        );
        // 不带单位的裸数字 = 块数。
        assert_eq!(
            parse_retention("18").unwrap(),
            (Some(Retention::Blocks(18)), String::new())
        );
        // 数字后面还可以跟 focus。
        assert_eq!(
            parse_retention("18 关注认证").unwrap(),
            (Some(Retention::Blocks(18)), "关注认证".to_string())
        );
        assert_eq!(
            parse_retention("20k  关注认证").unwrap(),
            (Some(Retention::Tokens(20_000)), "关注认证".to_string())
        );
        // 不是数字就整句都是 focus。
        assert_eq!(
            parse_retention("关注认证").unwrap(),
            (None, "关注认证".to_string())
        );
        assert_eq!(
            parse_retention("18x").unwrap(),
            (None, "18x".to_string())
        );
    }

    /// 敲了数字却没生效比报错更糟：解析不动就拒绝，不悄悄退回默认。
    #[test]
    fn retention_argument_refuses_what_it_cannot_honor() {
        assert!(parse_retention("0").is_err(), "零个块不是合法请求");
        assert!(
            parse_retention("99999999999999999999999").is_err(),
            "溢出的数字要报错，不能夹到某个上限"
        );
        assert!(parse_retention("99999999999999999999999k").is_err());
    }

    /// profile 的 `compact.md` 优先于 config 的 `instruction_file`。
    #[test]
    fn profile_instruction_outranks_the_config_file() {
        let dir = std::env::temp_dir().join(format!("mypi-compact-instr-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let profile = dir.join("profile.md");
        let config = dir.join("config.md");
        std::fs::write(&profile, "PROFILE 指令").unwrap();
        std::fs::write(&config, "CONFIG 指令").unwrap();

        let got = load_instruction(Some(&CompactSource::File(profile)), Some(&config)).unwrap();
        assert_eq!(got, "PROFILE 指令");
        // 没有 profile 文件时退回 config。
        assert_eq!(load_instruction(None, Some(&config)).unwrap(), "CONFIG 指令");
        // 两样都没有 = 内置。
        assert_eq!(load_instruction(None, None).unwrap(), DEFAULT_INSTRUCTION);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// profile 点名了一个读不到的 `compact.md` 要报错——静默滑回内置会用
    /// 用户没写过的提示词压缩，且一声不吭。
    #[test]
    fn a_named_but_unreadable_profile_instruction_fails_loudly() {
        let missing = std::path::Path::new("/nonexistent/mypi/compact.md");
        let e = load_instruction(Some(&CompactSource::File(missing.into())), None).unwrap_err();
        assert!(format!("{e:#}").contains("compact.md"), "{e:#}");
    }

    /// `{{focus}}` 的替换：空 focus 整行拿掉，非空则替换进去。
    #[test]
    fn focus_line_is_substituted_or_dropped() {
        let t = "前\n{{focus}}\n后";
        assert_eq!(render_instruction(t, "  "), "前\n后");
        let rendered = render_instruction(t, "认证");
        assert!(rendered.contains("认证"), "{rendered}");
        assert!(!rendered.contains("{{focus}}"), "{rendered}");
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

    /// `compact()` 真的按传进来的 retention 切：默认预算下压不动，换成块数
    /// 口径就能压——证明参数一路走到了 `cut`，而不是在 `run_compact` 里掉了。
    #[test]
    fn compact_honors_the_retention_argument() {
        let es: Vec<Entry> = (0..6).map(|_i| user(&"z".repeat(400))).collect();
        let cfg = CompactConfig {
            retain_tail: 10_000,
            ..Default::default()
        };
        let mut summarize = |_: &crate::server::ai::types::Context| -> anyhow::Result<String> { Ok("短".into()) };

        // 6 块共 2400 字符 ≈ 600 token，远在 10k 预算之内 = 全留，没得压。
        let untouched = compact("sys", &es, &cfg, None, None, "", &mut summarize);
        assert!(untouched.is_err(), "尾部覆盖了全部条目，应当拒绝");

        // 块数口径：留最新 2 块，前 4 块进总结。
        let out = compact(
            "sys",
            &es,
            &cfg,
            None,
            Some(Retention::Blocks(2)),
            "",
            &mut summarize,
        )
        .unwrap();
        match out.marker {
            Entry::Compaction {
                first_kept_entry, ..
            } => assert_eq!(first_kept_entry, 4),
            other => panic!("marker 应该是 Compaction，实际是 {other:?}"),
        }
    }

    /// profile 的指令路径也一路走到 `compact()`：读失败要在这一层就冒出来。
    #[test]
    fn compact_surfaces_an_unreadable_profile_instruction() {
        let es: Vec<Entry> = (0..3).map(|_i| user(&"y".repeat(400))).collect();
        let cfg = CompactConfig {
            retain_tail: 10,
            ..Default::default()
        };
        let mut summarize = |_: &crate::server::ai::types::Context| -> anyhow::Result<String> { Ok("短".into()) };
        let missing = std::path::Path::new("/nonexistent/mypi/compact.md");
        let e = compact(
            "sys",
            &es,
            &cfg,
            Some(&CompactSource::File(missing.into())),
            None,
            "",
            &mut summarize,
        )
        .unwrap_err();
        assert!(format!("{e:#}").contains("compact.md"), "{e:#}");
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
        let r = compact("sys", &es, &cfg, None, None, "", &mut summarize);
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
        let out = compact("sys", &es, &cfg, None, None, "", &mut summarize).unwrap();
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
                .filter(|m| matches!(m, Message::User { content } if content.text().starts_with('y')))
                .count();
            *shape.borrow_mut() = vec![req.messages.len(), users];
            Ok("## 当前工作\n\n无".into())
        };
        let _ = compact("sys", &es, &cfg, None, None, "", &mut summarize).unwrap();
        // system + 3 user turns + instruction; the region (2 turns) appears once.
        assert_eq!(*shape.borrow(), vec![5, 3]);
    }
}
