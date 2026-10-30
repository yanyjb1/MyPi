//! In-band parsing: some hosts provide no structured `tool_calls` / `reasoning_content`, so the model
//! writes calls and thinking inside the text. This turns that text back into structured events.

/// One event a scan produced.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ScanEvent {
    /// Visible text.
    Text(String),
    /// Thinking content (belongs in reasoning, not in the visible text).
    Thinking(String),
    /// One complete tool call.
    ToolCall {
        name: String,
        /// The raw argument text. Not parsed here: bad JSON is a different failure from no call at all.
        arguments: String,
    },
}

/// A state machine that recognises markers in text.
pub(crate) trait Scanner: Send {
    /// Feed a piece of text; emit what is now certain, hold the rest back.
    fn feed(&mut self, text: &str) -> Vec<ScanEvent>;
    /// The stream ended: emit everything held back. Must be called, or text is lost.
    fn flush(&mut self) -> Vec<ScanEvent>;
}

/// Pick a scanner for this host. `None` = nothing to parse (the structured fields are there).
pub(crate) fn scanner_for(
    compat: &crate::server::agent::request::protocol::completion::compat::Resolved,
) -> Option<Box<dyn Scanner>> {
    use crate::server::agent::request::protocol::completion::compat::MarkupHealing;
    match compat.stream_markup_healing_pattern? {
        MarkupHealing::Kimi => Some(Box::new(KimiScanner::new())),
        MarkupHealing::Dsml => Some(Box::new(XmlScanner::new())),
        MarkupHealing::Qwen => Some(Box::new(QwenXmlScanner::new())),
        MarkupHealing::Thinking => Some(Box::new(ThinkingScanner::new())),
    }
}

// ---- shared tail holding ----

/// How many trailing bytes of `text` could be the start of one of `markers`. Only shorter than a whole
/// marker counts: a complete one has already been recognised.
fn held_tail(text: &str, markers: &[&str]) -> usize {
    let mut best = 0;
    for m in markers {
        // Character-boundary prefixes of each marker, longest first: `&m[..n]` with an arbitrary byte
        // count would split a multi-byte character and panic.
        let prefix_lens: Vec<usize> = m
            .char_indices()
            .skip(1) // skip the empty prefix
            .map(|(i, _)| i)
            .chain(std::iter::once(m.len()))
            .collect();
        // A whole marker does not count as a possible start: it was already recognised.
        for &n in prefix_lens[..prefix_lens.len() - 1].iter().rev() {
            if n > text.len() || !text.is_char_boundary(text.len() - n) {
                continue;
            }
            if text.ends_with(&m[..n]) {
                best = best.max(n);
                break;
            }
        }
    }
    best
}

// ---- thinking fences ----

/// The thinking fences the chat templates use: bare open/close pairs with no attributes, which is what
/// `renderThinking` writes and what models leak.
const THINK_FENCES: &[(&str, &str)] = &[
    ("<think>", "</think>"),
    ("<thinking>", "</thinking>"),
    ("<scratchpad>", "</scratchpad>"),
    ("```thinking\n", "```"),
    ("<|channel>thought\n", "<channel|>"),
    (
        "<|start|>assistant<|channel|>analysis<|message|>",
        "<|end|>",
    ),
    ("<|channel|>analysis<|message|>", "<|end|>"),
];

struct ThinkingScanner {
    buf: String,
    /// The close marker while inside a thinking block.
    close: Option<&'static str>,
}

impl ThinkingScanner {
    fn new() -> Self {
        Self {
            buf: String::new(),
            close: None,
        }
    }

    /// The marker table: opens when looking for an open, closes when looking for a close.
    fn markers(&self) -> Vec<&'static str> {
        match self.close {
            Some(c) => vec![c],
            None => THINK_FENCES.iter().map(|(o, _)| *o).collect(),
        }
    }
}

impl Scanner for ThinkingScanner {
    fn feed(&mut self, text: &str) -> Vec<ScanEvent> {
        self.buf.push_str(text);
        let mut out = Vec::new();
        loop {
            match self.close {
                None => {
                    // The earliest open marker.
                    let hit = THINK_FENCES
                        .iter()
                        .filter_map(|(o, c)| self.buf.find(o).map(|i| (i, o.len(), *c)))
                        .min_by_key(|(i, _, _)| *i);
                    let Some((at, len, close)) = hit else {
                        // No open marker: emit what certainly is not one.
                        let hold = held_tail(&self.buf, &self.markers());
                        let cut = self.buf.len() - hold;
                        if cut == 0 {
                            return out;
                        }
                        out.push(ScanEvent::Text(self.buf[..cut].to_string()));
                        self.buf.drain(..cut);
                        return out;
                    };
                    if at > 0 {
                        out.push(ScanEvent::Text(self.buf[..at].to_string()));
                    }
                    self.buf.drain(..at + len);
                    self.close = Some(close);
                }
                Some(close) => {
                    let Some(at) = self.buf.find(close) else {
                        let hold = held_tail(&self.buf, &[close]);
                        let cut = self.buf.len() - hold;
                        if cut > 0 {
                            out.push(ScanEvent::Thinking(self.buf[..cut].to_string()));
                            self.buf.drain(..cut);
                        }
                        return out;
                    };
                    if at > 0 {
                        out.push(ScanEvent::Thinking(self.buf[..at].to_string()));
                    }
                    self.buf.drain(..at + close.len());
                    self.close = None;
                }
            }
        }
    }

    fn flush(&mut self) -> Vec<ScanEvent> {
        if self.buf.is_empty() {
            return Vec::new();
        }
        let held = std::mem::take(&mut self.buf);
        // An unclosed thinking block is emitted as thinking: it is truncated, not something else.
        match self.close.take() {
            Some(_) => vec![ScanEvent::Thinking(held)],
            None => vec![ScanEvent::Text(held)],
        }
    }
}

// ---- Kimi's chat-template tokens ----

const KIMI_SECTION_BEGIN: &str = "<|tool_calls_section_begin|>";
const KIMI_SECTION_END: &str = "<|tool_calls_section_end|>";
const KIMI_CALL_BEGIN: &str = "<|tool_call_begin|>";
const KIMI_CALL_END: &str = "<|tool_call_end|>";
const KIMI_ARG_BEGIN: &str = "<|tool_call_argument_begin|>";

const KIMI_TOKENS: &[&str] = &[
    KIMI_SECTION_BEGIN,
    KIMI_SECTION_END,
    KIMI_CALL_BEGIN,
    KIMI_CALL_END,
    KIMI_ARG_BEGIN,
];

/// Kimi carries tool calls in segments: `<|tool_calls_section_begin|> … <|tool_call_begin|>functions.read:0`
/// `<|tool_call_argument_begin|>{…}<|tool_call_end|>`. The name is what sits before the colon.
struct KimiScanner {
    buf: String,
    /// Text also goes through a thinking scan (Kimi leaks `<think>` into the same stream).
    thinking: ThinkingScanner,
}

impl KimiScanner {
    fn new() -> Self {
        Self {
            buf: String::new(),
            thinking: ThinkingScanner::new(),
        }
    }

    /// Text goes through the thinking scanner first: calls and thinking leak in one stream.
    fn via_thinking(&mut self, events: Vec<ScanEvent>) -> Vec<ScanEvent> {
        let mut out = Vec::new();
        for e in events {
            match e {
                ScanEvent::Text(t) => out.extend(self.thinking.feed(&t)),
                other => out.push(other),
            }
        }
        out
    }
}

/// `functions.read:0` / `read:0` / `read` → `read`。
fn kimi_function_name(raw: &str) -> String {
    let before_colon = raw.split(':').next().unwrap_or(raw);
    before_colon
        .rsplit('.')
        .next()
        .unwrap_or(before_colon)
        .trim()
        .to_string()
}

impl Scanner for KimiScanner {
    fn feed(&mut self, text: &str) -> Vec<ScanEvent> {
        self.buf.push_str(text);
        let mut out = Vec::new();
        loop {
            // Outside a call: find the next marker; whatever is safe goes out as text.
            let Some((at, _)) = KIMI_TOKENS
                .iter()
                .filter_map(|t| self.buf.find(t).map(|i| (i, *t)))
                .min_by_key(|(i, _)| *i)
            else {
                let hold = held_tail(&self.buf, KIMI_TOKENS);
                let cut = self.buf.len() - hold;
                if cut > 0 {
                    let text = self.buf[..cut].to_string();
                    self.buf.drain(..cut);
                    out.extend(self.via_thinking(vec![ScanEvent::Text(text)]));
                }
                return out;
            };
            if at > 0 {
                let text = self.buf[..at].to_string();
                self.buf.drain(..at);
                out.extend(self.via_thinking(vec![ScanEvent::Text(text)]));
                continue;
            }
            // Right at the start of a marker.
            if self.buf.starts_with(KIMI_SECTION_BEGIN) || self.buf.starts_with(KIMI_SECTION_END) {
                let n = if self.buf.starts_with(KIMI_SECTION_BEGIN) {
                    KIMI_SECTION_BEGIN.len()
                } else {
                    KIMI_SECTION_END.len()
                };
                self.buf.drain(..n);
                continue;
            }
            if !self.buf.starts_with(KIMI_CALL_BEGIN) && !self.buf.starts_with(KIMI_ARG_BEGIN) {
                // A lone `call_end`: that call is already complete.
                if self.buf.starts_with(KIMI_CALL_END) {
                    self.buf.drain(..KIMI_CALL_END.len());
                    continue;
                }
                // Unrecognised shape: emit it as text rather than getting stuck.
                let ch = self.buf.chars().next().expect("non-empty");
                self.buf.drain(..ch.len_utf8());
                out.extend(self.via_thinking(vec![ScanEvent::Text(ch.to_string())]));
                continue;
            }
            // A call starts here: `<|tool_call_begin|>name:idx<|tool_call_argument_begin|>{…}`.
            let rest = self
                .buf
                .strip_prefix(KIMI_CALL_BEGIN)
                .unwrap_or(&self.buf)
                .to_string();
            let Some(arg_at) = rest.find(KIMI_ARG_BEGIN) else {
                // Arguments have not arrived: hold the lot until the next chunk.
                return out;
            };
            let name = kimi_function_name(&rest[..arg_at]);
            let after = &rest[arg_at + KIMI_ARG_BEGIN.len()..];
            let Some(end_at) = after.find(KIMI_CALL_END) else {
                return out;
            };
            let arguments = after[..end_at].to_string();
            if name.is_empty() {
                // Every name empty: not a call that could run; emit it as text.
                out.extend(self.via_thinking(vec![ScanEvent::Text(format!("{name}{arguments}"))]));
            } else {
                out.push(ScanEvent::ToolCall { name, arguments });
            }
            let consumed = (self.buf.len() - rest.len())
                + arg_at
                + KIMI_ARG_BEGIN.len()
                + end_at
                + KIMI_CALL_END.len();
            self.buf.drain(..consumed);
        }
    }

    fn flush(&mut self) -> Vec<ScanEvent> {
        let mut out = Vec::new();
        if !self.buf.is_empty() {
            let held = std::mem::take(&mut self.buf);
            out.extend(self.via_thinking(vec![ScanEvent::Text(held)]));
        }
        out.extend(self.thinking.flush());
        out
    }
}

// ---- DSML (DeepSeek's XML envelope) ----

/// DeepSeek's DSML envelope, in both spellings (full-width `｜` and ASCII `|`):
/// `<｜DSML｜tool_calls><｜DSML｜invoke name="read">…</｜DSML｜invoke></｜DSML｜tool_calls>`.
struct XmlScanner {
    buf: String,
    thinking: ThinkingScanner,
}

impl XmlScanner {
    fn new() -> Self {
        Self {
            buf: String::new(),
            thinking: ThinkingScanner::new(),
        }
    }

    fn via_thinking(&mut self, events: Vec<ScanEvent>) -> Vec<ScanEvent> {
        let mut out = Vec::new();
        for e in events {
            match e {
                ScanEvent::Text(t) => out.extend(self.thinking.feed(&t)),
                other => out.push(other),
            }
        }
        out
    }
}

/// Full-width `｜` → ASCII `|`, so only one set of markers has to be known.
fn normalize_pipes(s: &str) -> String {
    s.replace('\u{FF5C}', "|")
}

impl Scanner for XmlScanner {
    fn feed(&mut self, text: &str) -> Vec<ScanEvent> {
        self.buf.push_str(text);
        let mut out = Vec::new();
        loop {
            // Every search and slice happens on `self.buf` itself: after normalisation the byte offsets
            // no longer line up, and slicing would panic or silently cut the wrong bytes.
            const OPEN_ASCII: &str = "<|DSML|tool_calls>";
            const OPEN_FULL: &str = "<\u{FF5C}DSML\u{FF5C}tool_calls>";
            let open_at = self
                .buf
                .find(OPEN_ASCII)
                .into_iter()
                .chain(self.buf.find(OPEN_FULL))
                .min();
            let Some(start) = open_at else {
                // No envelope: emit what is not a marker start; the hold check runs on the original too.
                let hold = held_tail(&self.buf, &[OPEN_ASCII, OPEN_FULL]);
                let cut = self.buf.len() - hold;
                if cut == 0 {
                    return out;
                }
                let text = self.buf[..cut].to_string();
                self.buf.drain(..cut);
                out.extend(self.via_thinking(vec![ScanEvent::Text(text)]));
                return out;
            };
            if start > 0 {
                let text = self.buf[..start].to_string();
                self.buf.drain(..start);
                out.extend(self.via_thinking(vec![ScanEvent::Text(text)]));
                continue;
            }
            let open_len = if self.buf.starts_with(OPEN_FULL) {
                OPEN_FULL.len()
            } else {
                OPEN_ASCII.len()
            };
            let close_ascii = "</|DSML|tool_calls>";
            // The close marker is looked up in the original, both spellings, earliest wins.
            let Some(close_at) = close_in(&self.buf, close_ascii) else {
                return out; // the envelope is not closed yet: wait for more
            };
            let close_len =
                if self.buf[close_at..].starts_with(&close_ascii.replace('|', "\u{FF5C}")) {
                    close_ascii.replace('|', "\u{FF5C}").len()
                } else {
                    close_ascii.len()
                };
            let body = self.buf[open_len..close_at].to_string();
            out.extend(parse_dsml_invocations(&body));
            self.buf.drain(..close_at + close_len);
        }
    }

    fn flush(&mut self) -> Vec<ScanEvent> {
        let mut out = Vec::new();
        if !self.buf.is_empty() {
            let held = std::mem::take(&mut self.buf);
            out.extend(self.via_thinking(vec![ScanEvent::Text(held)]));
        }
        out.extend(self.thinking.flush());
        out
    }
}

/// Find the earliest of the two DSML close spellings in the original text.
fn close_in(s: &str, ascii: &str) -> Option<usize> {
    let full = ascii.replace('|', "\u{FF5C}");
    match (s.find(ascii), s.find(&full)) {
        (Some(a), Some(f)) => Some(a.min(f)),
        (a, f) => a.or(f),
    }
}

/// Pull every `invoke` out of one DSML envelope body.
fn parse_dsml_invocations(body: &str) -> Vec<ScanEvent> {
    let flat = normalize_pipes(body);
    let mut out = Vec::new();
    let mut rest = flat.as_str();
    while let Some(at) = rest.find("<|DSML|invoke ") {
        let Some(tag_end) = rest[at..].find('>').map(|i| at + i) else {
            break;
        };
        let open_tag = &rest[at..=tag_end];
        let Some(name) = attr_value(open_tag, "name") else {
            rest = &rest[tag_end + 1..];
            continue;
        };
        let Some(close) = rest[tag_end..].find("</|DSML|invoke>") else {
            break;
        };
        let inner = &rest[tag_end + 1..tag_end + close];
        let args = parse_dsml_parameters(inner);
        out.push(ScanEvent::ToolCall {
            name,
            arguments: args,
        });
        rest = &rest[tag_end + close + "</|DSML|invoke>".len()..];
    }
    out
}

fn parse_dsml_parameters(inner: &str) -> String {
    let mut map = serde_json::Map::new();
    let mut rest = inner;
    while let Some(at) = rest.find("<|DSML|parameter ") {
        let Some(tag_end) = rest[at..].find('>').map(|i| at + i) else {
            break;
        };
        let open_tag = &rest[at..=tag_end];
        let Some(name) = attr_value(open_tag, "name") else {
            rest = &rest[tag_end + 1..];
            continue;
        };
        let Some(close) = rest[tag_end..].find("</|DSML|parameter>") else {
            break;
        };
        let raw = &rest[tag_end + 1..tag_end + close];
        // `string="true"` keeps it as text; otherwise parse it as JSON once.
        let is_string = attr_value(open_tag, "string").as_deref() == Some("true");
        let value = if is_string {
            serde_json::Value::String(raw.to_string())
        } else {
            serde_json::from_str(raw).unwrap_or_else(|_| serde_json::Value::String(raw.to_string()))
        };
        map.insert(name, value);
        rest = &rest[tag_end + close + "</|DSML|parameter>".len()..];
    }
    serde_json::Value::Object(map).to_string()
}

/// Read one attribute from an open tag, either quote style.
fn attr_value(tag: &str, attr: &str) -> Option<String> {
    let needle = format!("{attr}=");
    let at = tag.find(&needle)? + needle.len();
    let rest = &tag[at..];
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let end = rest[1..].find(quote)? + 1;
    Some(rest[1..end].to_string())
}

// ---- Qwen's `<tool_calls>` XML ----

/// Qwen's chat templates (vLLM / DashScope) use a simpler XML: `<tool_calls><tool_call><function=read>`
/// `<parameter=path>a.rs</parameter></function></tool_call></tool_calls>`.
struct QwenXmlScanner {
    buf: String,
    thinking: ThinkingScanner,
}

impl QwenXmlScanner {
    fn new() -> Self {
        Self {
            buf: String::new(),
            thinking: ThinkingScanner::new(),
        }
    }

    fn via_thinking(&mut self, events: Vec<ScanEvent>) -> Vec<ScanEvent> {
        let mut out = Vec::new();
        for e in events {
            match e {
                ScanEvent::Text(t) => out.extend(self.thinking.feed(&t)),
                other => out.push(other),
            }
        }
        out
    }
}

impl Scanner for QwenXmlScanner {
    fn feed(&mut self, text: &str) -> Vec<ScanEvent> {
        self.buf.push_str(text);
        let mut out = Vec::new();
        loop {
            let Some(at) = self.buf.find("<tool_calls>") else {
                let hold = held_tail(&self.buf, &["<tool_calls>"]);
                let cut = self.buf.len() - hold;
                if cut == 0 {
                    return out;
                }
                let text = self.buf[..cut].to_string();
                self.buf.drain(..cut);
                out.extend(self.via_thinking(vec![ScanEvent::Text(text)]));
                return out;
            };
            if at > 0 {
                let text = self.buf[..at].to_string();
                self.buf.drain(..at);
                out.extend(self.via_thinking(vec![ScanEvent::Text(text)]));
                continue;
            }
            let Some(close) = self.buf.find("</tool_calls>") else {
                return out; // not closed yet: wait for the next chunk
            };
            let body = self.buf["<tool_calls>".len()..close].to_string();
            out.extend(parse_qwen_calls(&body));
            self.buf.drain(..close + "</tool_calls>".len());
        }
    }

    fn flush(&mut self) -> Vec<ScanEvent> {
        let mut out = Vec::new();
        if !self.buf.is_empty() {
            let held = std::mem::take(&mut self.buf);
            out.extend(self.via_thinking(vec![ScanEvent::Text(held)]));
        }
        out.extend(self.thinking.flush());
        out
    }
}

fn parse_qwen_calls(body: &str) -> Vec<ScanEvent> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(at) = rest.find("<function=") {
        let name_start = at + "<function=".len();
        let Some(name_end) = rest[name_start..].find('>').map(|i| name_start + i) else {
            break;
        };
        let name = rest[name_start..name_end].trim().to_string();
        let Some(close) = rest[name_end..].find("</function>") else {
            break;
        };
        let inner = &rest[name_end + 1..name_end + close];
        let mut map = serde_json::Map::new();
        let mut p = inner;
        while let Some(pat) = p.find("<parameter=") {
            let key_start = pat + "<parameter=".len();
            let Some(key_end) = p[key_start..].find('>').map(|i| key_start + i) else {
                break;
            };
            let key = p[key_start..key_end].trim().to_string();
            let Some(pclose) = p[key_end..].find("</parameter>") else {
                break;
            };
            let value = &p[key_end + 1..key_end + pclose];
            map.insert(key, serde_json::Value::String(value.to_string()));
            p = &p[key_end + pclose + "</parameter>".len()..];
        }
        if !name.is_empty() {
            out.push(ScanEvent::ToolCall {
                name,
                arguments: serde_json::Value::Object(map).to_string(),
            });
        }
        rest = &rest[name_end + close + "</function>".len()..];
    }
    out
}
