//! Strips leaked chat-template markers from visible text. A pure text transform: a chunk in, the
//! emittable part out; the tail is held back while it could still become a marker.

/// Strips leaked chat-template markers (`<｜…｜>` / `<|…|>` envelopes, and whole angle tags) from text.
#[derive(Default)]
pub(crate) struct TemplateMarkupFilter {
    /// Bytes held back from the previous chunk.
    pub(crate) held: String,
    /// Strip `<｜ … ｜>` / `<| … |>` envelopes (DeepSeek's DSML, Kimi's section and call markers).
    pub(crate) pipes: bool,
    /// Which angle tags to strip, e.g. `[["<think>", "</think>"]]`; an empty array strips none.
    pub(crate) tags: &'static [&'static [&'static str]],
}

/// The angle tags each pattern needs. Only what must go: `<think>` fences and `<tool_call>` copies of
/// a structured field. Ordinary angle brackets are never touched.
const KIMI_TAGS: &[&[&str]] = &[&["<think>", "</think>"]];
const QWEN_TAGS: &[&[&str]] = &[&["<tool_call>", "</tool_call>"], &["<think>", "</think>"]];
const THINKING_TAGS: &[&[&str]] = &[&["<thinking>", "</thinking>"]];

impl TemplateMarkupFilter {
    /// Build one for this host's switches; the default (nothing on) is a straight `push_str`.
    pub(crate) fn for_compat(
        c: &crate::server::agent::request::protocol::completion::compat::Resolved,
    ) -> Self {
        use crate::server::agent::request::protocol::completion::compat::MarkupHealing;
        Self {
            held: String::new(),
            pipes: c.strip_deepseek_special_tokens,
            tags: match c.stream_markup_healing_pattern {
                Some(MarkupHealing::Kimi) => KIMI_TAGS,
                Some(MarkupHealing::Qwen) => QWEN_TAGS,
                Some(MarkupHealing::Thinking) => THINKING_TAGS,
                // DSML is a `｜` envelope, handled by `pipes`, not an angle tag.
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

/// Split `text` into (safe to emit now, must hold back). Complete markers are removed; a trailing
/// sequence that could still become one is held back.
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
                // No close yet: hold the tail, unless it is already too long to be a marker.
                if rest.len() <= MAX_HELD_MARKER_BYTES {
                    return (out, rest.to_string());
                }
            }
            // Whole angle tags drop out, the text between them stays: it is the model's own words.
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
                && tags.iter().any(|g| g.iter().any(|t| t.starts_with(rest)))
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
        if !(ch.is_alphanumeric() || ch == '_' || ch == '｜' || ch == '|' || ch == '▁' || ch == '.')
        {
            return None;
        }
    }
    None
}

impl TemplateMarkupFilter {
    /// Feed one chunk; what can be emitted is appended to `out`. A chunk that is all half-marker waits.
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
