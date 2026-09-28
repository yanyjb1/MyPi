//! 从可见文本里剥掉泄漏的模板标记。
//!
//! 纯文本变换：给定一片文本，吐出（现在能发的，必须扣住的）。不碰网络、
//! 不碰 `Client`、不读任何配置——`enabled` 由调用方从 compat 里取。

/// Strips leaked chat-template markers from a visible text stream.
///
/// # What it is for
///
/// DeepSeek-family models hosted behind providers that do not strip template
/// tokens server-side (NVIDIA NIM and various gateways) leak the raw special
/// tokens into `content`:
///
/// ```text
/// <｜tool_calls_begin｜>…<｜tool_calls_end｜>
/// <｜DSML｜tool_calls｜>
/// ```
///
/// The structured `tool_calls` field is still emitted correctly — only the
/// visible text is polluted. Dropping these therefore costs nothing.
///
/// # Why it holds bytes back
///
/// A marker gets **split across chunks**: one chunk ends with `<｜tool_calls`,
/// the next starts with `_begin｜>`. Matching chunk by chunk lets both halves
/// through (neither matches the full pattern on its own) and the user sees
/// half a marker. So the tail is retained whenever it could still become a
/// marker, and re-examined when the next chunk arrives.
///
/// # What it will not eat
///
/// Only `<` immediately followed by `｜` (U+FF5C) or `|` qualifies as the start
/// of a marker, and the body must be identifier-like. `<think>`, `<div>`, and
/// ordinary angle-bracket prose are untouched, so this is safe to enable on any
/// host.
#[derive(Default)]
pub(crate) struct TemplateMarkupFilter {
    /// Bytes held back from the previous chunk.
    pub(crate) held: String,
    /// 剥 `<｜ … ｜>` / `<| … |>` 信封（DeepSeek 的 DSML、Kimi 的
    /// section / call 标记都是这个形状）。
    pub(crate) pipes: bool,
    /// 剥哪几组角度标签，如 `[["<think>", "</think>"]]`。
    ///
    /// 按宿主挑 `stream_markup_healing_pattern`，见 [`Self::for_compat`]。
    /// 空数组 = 不剥任何角度标签。
    pub(crate) tags: &'static [&'static [&'static str]],
}

/// 各 pattern 对应的角度标签组。
///
/// 只列**必须剥掉**的：`<think>` 那种围栏是思考内容被塞进可见文本，
/// `<tool_call>` 是结构化字段之外又复制一份 JSON。普通尖括号（`<div>`、
/// `a < b`）永远不动——见 `ordinary_angle_brackets_pass_through_untouched`。
const KIMI_TAGS: &[&[&str]] = &[&["<think>", "</think>"]];
const QWEN_TAGS: &[&[&str]] = &[
    &["<tool_call>", "</tool_call>"],
    &["<think>", "</think>"],
];
const THINKING_TAGS: &[&[&str]] = &[&["<thinking>", "</thinking>"]];

impl TemplateMarkupFilter {
    /// 按这个宿主的开关造一个。默认（什么都不开）就是一条直通的 `push_str`。
    pub(crate) fn for_compat(c: &crate::server::ai::compat::Resolved) -> Self {
        use crate::server::ai::compat::MarkupHealing;
        Self {
            held: String::new(),
            pipes: c.strip_deepseek_special_tokens,
            tags: match c.stream_markup_healing_pattern {
                Some(MarkupHealing::Kimi) => KIMI_TAGS,
                Some(MarkupHealing::Qwen) => QWEN_TAGS,
                Some(MarkupHealing::Thinking) => THINKING_TAGS,
                // DSML 就是 `｜` 信封，靠 `pipes` 处理，不是角度标签。
                Some(MarkupHealing::Dsml) | None => &[],
            },
        }
    }

    fn is_on(&self) -> bool {
        self.pipes || !self.tags.is_empty()
    }
}

/// Cap on a held tail. A real marker body is short; anything longer is
/// ordinary text that merely started with `<|`.
const MAX_HELD_MARKER_BYTES: usize = 64;

/// Split `text` into (safe to emit now, must hold back).
///
/// Complete markers are removed outright: `<｜ … ｜>` / `<| … |>` envelopes when
/// `pipes` is on, and any whole tag from `tags`. A trailing sequence that could
/// still become one of either (`<｜`, `<|`, `<｜DSML｜tool`, `</thi`) is held
/// back — emitting it would leak half a marker if it turns out to be one, and
/// dropping it would lose real text if it does not.
///
/// `pipes` is a parameter rather than an unconditional rule because the pipe
/// envelope is **not** universal: `<|im_end|>` is ordinary text to a host that
/// never emitted one, and eating it would be silently rewriting the model.
fn split_template_markup(text: &str, pipes: bool, tags: &[&[&str]]) -> (String, String) {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // A marker opens with `<` + fullwidth pipe or ASCII pipe.
        if bytes[i] == b'<' {
            let rest = &text[i..];
            let mut chars = rest.chars();
            chars.next(); // the '<'
            let after = chars.next();
            let opens = pipes && matches!(after, Some('\u{ff5c}') | Some('|'));
            if opens {
                // Look for the close: `｜>` or `|>`.
                if let Some(close) = find_marker_close(rest) {
                    i += close; // skip the whole marker
                    continue;
                }
                // No close yet. Hold the tail — unless it is already too long
                // to be a marker, in which case it is ordinary text.
                //
                // `out` already holds everything before `i` (the loop pushed it
                // character by character), so nothing needs to be re-appended
                // here. Doing so duplicates the prefix and the user sees the
                // start of the reply twice.
                if rest.len() <= MAX_HELD_MARKER_BYTES {
                    return (out, rest.to_string());
                }
            }
            // Whole angle tags drop out, the text between them stays: the JSON
            // inside a `<tool_call>` is the model's own words, and deleting it
            // would be putting words in its mouth.
            let mut dropped = false;
            for group in tags {
                for tag in *group {
                    if rest.starts_with(tag) {
                        i += tag.len();
                        dropped = true;
                        break;
                    }
                }
                if dropped {
                    break;
                }
            }
            if dropped {
                continue;
            }
            // `</thi` could still become `</think>` — hold it.
            if rest.len() <= MAX_HELD_MARKER_BYTES
                && tags
                    .iter()
                    .any(|g| g.iter().any(|t| t.starts_with(rest)))
            {
                return (out, rest.to_string());
            }
        }
        // Copy one whole character (never split a UTF-8 sequence).
        let ch_len = text[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        out.push_str(&text[i..i + ch_len]);
        i += ch_len;
    }
    (out, String::new())
}

/// Byte length of the marker starting at `rest`, or `None` if it never closes
/// within a plausible body length.
fn find_marker_close(rest: &str) -> Option<usize> {
    let mut chars = rest.char_indices();
    chars.next(); // '<'
    chars.next(); // '｜' or '|'
    for (idx, ch) in chars {
        if ch == '>' {
            let len = idx + 1;
            return (len <= MAX_HELD_MARKER_BYTES).then_some(len);
        }
        // Identifier-like bodies only; anything else means this was not a
        // template marker after all.
        if !(ch.is_alphanumeric() || ch == '_' || ch == '｜' || ch == '|' || ch == '▁' || ch == '.') {
            return None;
        }
    }
    None
}

impl TemplateMarkupFilter {
    /// Feed one chunk; the emitted text (markers removed, tail withheld) is
    /// appended to `out`. Nothing is emitted for a chunk that is entirely a
    /// half marker — it waits for the next one.
    pub(crate) fn feed(&mut self, text: &str, out: &mut String) {
        if !self.is_on() {
            out.push_str(text);
            return;
        }
        let mut buf = std::mem::take(&mut self.held);
        buf.push_str(text);
        let (emit, hold) = split_template_markup(&buf, self.pipes, self.tags);
        out.push_str(&emit);
        self.held = hold;
    }

    /// End of stream: the held tail never became a marker, so it was real text
    /// after all. Emit it instead of dropping the user's last few bytes.
    pub(crate) fn flush(&mut self, out: &mut String) {
        if !self.held.is_empty() {
            out.push_str(&self.held);
            self.held.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DeepSeek 系模板标记会被**切断**：一半在上一片、一半在这一片。
    /// 逐片匹配会让两半都漏出去，所以尾巴要扣住等下一片。
    #[test]
    fn leaked_template_markers_are_stripped_across_chunk_boundaries() {
        let mut f = TemplateMarkupFilter { pipes: true, ..Default::default() };
        let mut out = String::new();
        f.feed("好的<｜tool_calls", &mut out);
        assert_eq!(out, "好的", "半截标记不许发出去");
        f.feed("_begin｜>继续", &mut out);
        assert_eq!(out, "好的继续");
        assert_eq!(f.held, "");
    }

    /// 扣住的尾巴如果最后没变成标记，那是真文本，必须吐出来——不能吞用户
    /// 回复的最后几个字节。
    #[test]
    fn a_held_tail_that_never_closes_is_flushed_not_swallowed() {
        let mut f = TemplateMarkupFilter { pipes: true, ..Default::default() };
        let mut out = String::new();
        f.feed("3 <|4", &mut out);
        assert_eq!(out, "3 ");
        f.flush(&mut out);
        assert_eq!(out, "3 <|4");
    }

    /// 普通尖括号文本一个字符都不能少。`<think>`、`<div>`、`a < b` 都不是
    /// 标记——只有 `<` 紧跟全角竖线或 ASCII 竖线才算。
    #[test]
    fn ordinary_angle_brackets_pass_through_untouched() {
        for text in ["a < b", "<think>x</think>", "<div class=\"x\">", "1<2"] {
            let (emit, hold) = split_template_markup(text, true, &[]);
            assert_eq!(emit, text, "{text:?} 被误伤");
            assert_eq!(hold, "", "{text:?} 被误扣");
        }
    }


    // ---- 按宿主挑形状 ----

    use crate::server::ai::compat::{MarkupHealing, Resolved};

    fn strip(text: &str, c: &Resolved) -> String {
        let mut f = TemplateMarkupFilter::for_compat(c);
        let mut out = String::new();
        f.feed(text, &mut out);
        f.flush(&mut out);
        out
    }

    /// 默认宿主什么都不剥：多剥一个字符就是改模型的输出。
    #[test]
    fn a_plain_host_strips_nothing_at_all() {
        let r = Resolved::conservative();
        let text = "<think>想一下</think> <tool_call>{}</tool_call> <｜DSML｜tool_calls｜>";
        assert_eq!(strip(text, &r), text);
    }

    /// Qwen：`<tool_call>` 和 `<think>` 的**标签**掉，里面的字留下——
    /// 那是模型自己说的话，删掉等于替它说话。
    #[test]
    fn qwen_style_tags_go_but_their_contents_stay() {
        let mut r = Resolved::conservative();
        r.stream_markup_healing_pattern = Some(MarkupHealing::Qwen);
        assert_eq!(
            strip("好的<tool_call>{\"a\":1}</tool_call>继续", &r),
            "好的{\"a\":1}继续"
        );
        assert_eq!(strip("前<think>想</think>后", &r), "前想后");
        // 这套 pattern 不管 `｜` 信封。
        assert_eq!(strip("<|im_end|>", &r), "<|im_end|>");
    }

    /// Kimi：只剥 `<think>`，不碰 `<tool_call>`（它的工具调用走另一套标记）。
    #[test]
    fn kimi_only_drops_think_tags() {
        let mut r = Resolved::conservative();
        r.stream_markup_healing_pattern = Some(MarkupHealing::Kimi);
        assert_eq!(strip("前<think>想</think>后", &r), "前想后");
        assert_eq!(strip("<tool_call>x</tool_call>", &r), "<tool_call>x</tool_call>");
    }

    /// 通用思考围栏。
    #[test]
    fn the_generic_fence_pattern_drops_thinking_blocks() {
        let mut r = Resolved::conservative();
        r.stream_markup_healing_pattern = Some(MarkupHealing::Thinking);
        assert_eq!(strip("答<thinking>内</thinking>案", &r), "答内案");
    }

    /// DeepSeek 的 DSML 是 `｜` 信封，由另一个开关打开；两个开关互不越界。
    #[test]
    fn the_dsml_pattern_drops_envelopes_not_angle_tags() {
        let mut r = Resolved::conservative();
        r.stream_markup_healing_pattern = Some(MarkupHealing::Dsml);
        r.strip_deepseek_special_tokens = true;
        assert_eq!(strip("<｜DSML｜tool_calls｜>真话", &r), "真话");
        assert_eq!(strip("<think>x</think>", &r), "<think>x</think>");
    }

    /// 半截标签跨 chunk 到达时也要扣住：逐片匹配会让 `</thi` 和 `nk>` 都漏出去。
    #[test]
    fn a_split_angle_tag_is_held_back_until_it_completes() {
        let mut r = Resolved::conservative();
        r.stream_markup_healing_pattern = Some(MarkupHealing::Thinking);
        let mut f = TemplateMarkupFilter::for_compat(&r);
        let mut out = String::new();
        f.feed("答案<think", &mut out);
        assert_eq!(out, "答案", "半截标签必须扣住");
        f.feed("ing>别看我</thinking>好了", &mut out);
        assert_eq!(out, "答案别看我好了");
    }
}
