//! 带内（in-band）内容解析。
//!
//! # 这是干什么的
//!
//! 有的宿主**不提供**结构化的 `tool_calls` / `reasoning_content` 字段。模型
//! 只能把工具调用和思考过程**塞进正文文本**里：
//!
//! ```text
//! 我先看看这个文件。
//! <|tool_calls_section_begin|><|tool_call_begin|>read:0<|tool_call_argument_begin|>{"path":"a.rs"}<|tool_call_end|><|tool_calls_section_end|>
//! ```
//!
//! 不解析的话，这段标记要么原样显示给用户，要么（被 [`crate::server::ai::stream_patch`]
//! 剥掉之后）**工具根本不会执行**——模型看到自己调了工具却拿不到结果，于是
//! 重复输出或者开始编。钱和时间都白花，而且看起来像"模型变笨了"。
//!
//! 所以这一层做的是**文本 → 结构化事件**的翻译：把正文切成
//! [`ScanEvent::Text`] / [`ScanEvent::Thinking`] / [`ScanEvent::ToolCall`]。
//!
//! # 为什么可以分块喂
//!
//! 标记会**跨 chunk 裂开**：一个 chunk 以 `<|tool_call` 结尾，下一个以
//! `_begin|>` 开头。所以每个 scanner 都要扣住"尾巴上还可能是标记开头"的那
//! 几个字节，等下一个 chunk 到了再一起看。这是这一层最容易写错的地方——按
//! chunk 逐个匹配会让两个半截都漏过去。
//!
//! # 和 `stream_patch` 的分工
//!
//! `stream_patch` 只**剥**（不问剥掉的是什么）。这一层**解析**（把剥出来的
//! 内容变成能执行的东西）。落到 `stream_once` 里的顺序是：先过 `stream_patch`
//! 的标记过滤，再过这里的 scanner——不过实际上 scanner 自己的状态机能吃掉
//! 它认识的标记，`stream_patch` 是给"宿主声明了要剥、但 scanner 不认那个
//! 形状"的情况兜底的。

/// 一次扫描吐出来的东西。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ScanEvent {
    /// 可见正文。
    Text(String),
    /// 思考内容（该进 reasoning，不该显示成正文）。
    Thinking(String),
    /// 一个完整的工具调用。
    ToolCall {
        name: String,
        /// 参数原文（JSON 串）。**不在这里解析成对象**：模型可能发不合法的
        /// JSON，而"发不出去"和"发出去但参数坏了"是两种不同的失败，要让
        /// 下游分得开。
        arguments: String,
    },
}

/// 一个"从正文里认标记"的状态机。
pub(crate) trait Scanner: Send {
    /// 喂一段正文，吐出现在能确定的事件。没认出来的尾巴扣住。
    fn feed(&mut self, text: &str) -> Vec<ScanEvent>;
    /// 流结束：把所有扣住的内容吐出来。**必须调用**——不调用就丢文本。
    fn flush(&mut self) -> Vec<ScanEvent>;
}

/// 按宿主挑一个 scanner。`None` = 这个宿主不需要解析（结构化的字段都在）。
pub(crate) fn scanner_for(
    compat: &crate::server::ai::compat::Resolved,
) -> Option<Box<dyn Scanner>> {
    use crate::server::ai::compat::MarkupHealing;
    match compat.stream_markup_healing_pattern? {
        MarkupHealing::Kimi => Some(Box::new(KimiScanner::new())),
        MarkupHealing::Dsml => Some(Box::new(XmlScanner::new())),
        MarkupHealing::Qwen => Some(Box::new(QwenXmlScanner::new())),
        MarkupHealing::Thinking => Some(Box::new(ThinkingScanner::new())),
    }
}

// ---------------------------------------------------------------------------
// 通用的尾巴扣留
// ---------------------------------------------------------------------------

/// `text` 的末尾有多少字节可能是 `markers` 里某个标记的**开头**。
///
/// 只会返回比标记本身短的量：完整的标记不算"可能的开头"（它已经被认出来
/// 了）。0 表示尾巴肯定不是任何标记的开头，可以安全发出去。
fn held_tail(text: &str, markers: &[&str]) -> usize {
    let mut best = 0;
    for m in markers {
        let max = text.len().min(m.len().saturating_sub(1));
        // 从长到短试：取最长的那个"能接上"的前缀。
        for n in (1..=max).rev() {
            if !text.is_char_boundary(text.len() - n) {
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

// ---------------------------------------------------------------------------
// 思考围栏
// ---------------------------------------------------------------------------

/// 各家 chat template 的思考围栏。
///
/// 一律是**裸的、不带属性的**开闭对——那是 `renderThinking` 写出去的形状，
/// 也是模型实际泄漏的形状。带属性或带命名空间的（`<thinking signature="…">`）
/// 不在这一层处理。
const THINK_FENCES: &[(&str, &str)] = &[
    ("<think>", "</think>"),
    ("<thinking>", "</thinking>"),
    ("<scratchpad>", "</scratchpad>"),
    ("```thinking\n", "```"),
    ("<|channel>thought\n", "<channel|>"),
    ("<|start|>assistant<|channel|>analysis<|message|>", "<|end|>"),
    ("<|channel|>analysis<|message|>", "<|end|>"),
];

struct ThinkingScanner {
    buf: String,
    /// 正在一个思考块里时的闭合标记。
    close: Option<&'static str>,
}

impl ThinkingScanner {
    fn new() -> Self {
        Self {
            buf: String::new(),
            close: None,
        }
    }

    /// 标记表：在找开标记时看开标记，在找闭标记时只看闭标记。
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
                    // 找最早出现的开标记。
                    let hit = THINK_FENCES
                        .iter()
                        .filter_map(|(o, c)| self.buf.find(o).map(|i| (i, o.len(), *c)))
                        .min_by_key(|(i, _, _)| *i);
                    let Some((at, len, close)) = hit else {
                        // 没有开标记：把确定不是标记开头的部分发出去。
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
        // 没闭合的思考块**当思考发**：它确实是思考，只是被截断了。
        match self.close.take() {
            Some(_) => vec![ScanEvent::Thinking(held)],
            None => vec![ScanEvent::Text(held)],
        }
    }
}

// ---------------------------------------------------------------------------
// Kimi 的 chat template token
// ---------------------------------------------------------------------------

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

/// Kimi 的工具调用是**分段**的：
///
/// ```text
/// <|tool_calls_section_begin|>
///   <|tool_call_begin|>functions.read:0<|tool_call_argument_begin|>{"path":"a"}
///   <|tool_call_end|>
/// <|tool_calls_section_end|>
/// ```
///
/// 函数名的形状是 `functions.<name>:<index>`（有的宿主省掉 `functions.` 前缀，
/// 有的把 index 省掉），取冒号前、最后一个点之后的那一段。
struct KimiScanner {
    buf: String,
    /// 正文之外还套一层思考扫描（Kimi 也会在同一个流里泄漏 `<think>`）。
    thinking: ThinkingScanner,
}

impl KimiScanner {
    fn new() -> Self {
        Self {
            buf: String::new(),
            thinking: ThinkingScanner::new(),
        }
    }

    /// 正文过一遍思考扫描器：Kimi 会把工具标记和思考泄漏在同一个流里。
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
            // 段外：找下一个工具标记；没有就把安全的部分当正文。
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
            // 正好在一个标记的开头。
            if self.buf.starts_with(KIMI_SECTION_BEGIN) || self.buf.starts_with(KIMI_SECTION_END)
            {
                let n = if self.buf.starts_with(KIMI_SECTION_BEGIN) {
                    KIMI_SECTION_BEGIN.len()
                } else {
                    KIMI_SECTION_END.len()
                };
                self.buf.drain(..n);
                continue;
            }
            if !self.buf.starts_with(KIMI_CALL_BEGIN) && !self.buf.starts_with(KIMI_ARG_BEGIN) {
                // 单个的 `call_end`：一个调用已经收完了，丢掉。
                if self.buf.starts_with(KIMI_CALL_END) {
                    self.buf.drain(..KIMI_CALL_END.len());
                    continue;
                }
                // 不认识的形状：当正文吐出去，别卡住。
                let ch = self.buf.chars().next().expect("non-empty");
                self.buf.drain(..ch.len_utf8());
                out.extend(self.via_thinking(vec![ScanEvent::Text(ch.to_string())]));
                continue;
            }
            // 从这里开始是一个调用：`<|tool_call_begin|>name:idx<|tool_call_argument_begin|>{…}<|tool_call_end|>`
            let rest = self
                .buf
                .strip_prefix(KIMI_CALL_BEGIN)
                .unwrap_or(&self.buf)
                .to_string();
            let Some(arg_at) = rest.find(KIMI_ARG_BEGIN) else {
                // 参数还没到：扣住整段等下一个 chunk。
                return out;
            };
            let name = kimi_function_name(&rest[..arg_at]);
            let after = &rest[arg_at + KIMI_ARG_BEGIN.len()..];
            let Some(end_at) = after.find(KIMI_CALL_END) else {
                return out;
            };
            let arguments = after[..end_at].to_string();
            if name.is_empty() {
                // 名字都是空的：这不是能执行的调用，当正文吐出去（用户至少
                // 能看到模型写了什么），别产出一个必然失败的调用。
                out.extend(self.via_thinking(vec![ScanEvent::Text(format!(
                    "{name}{arguments}"
                ))]));
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

// ---------------------------------------------------------------------------
// DSML（DeepSeek 的 XML 信封）
// ---------------------------------------------------------------------------

/// DeepSeek 的 DSML 信封，两种拼法都要认：全角竖线 `｜` 与 ASCII 竖线 `|`。
///
/// 形状：
///
/// ```text
/// <｜DSML｜tool_calls>
///   <｜DSML｜invoke name="read">
///     <｜DSML｜parameter name="path" string="true">a.rs</｜DSML｜parameter>
///   </｜DSML｜invoke>
/// </｜DSML｜tool_calls>
/// ```
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

/// 把全角的 `｜` 换成 ASCII `|`，这样只需要认一套标记。
fn normalize_pipes(s: &str) -> String {
    s.replace('\u{FF5C}', "|")
}

impl Scanner for XmlScanner {
    fn feed(&mut self, text: &str) -> Vec<ScanEvent> {
        self.buf.push_str(text);
        let mut out = Vec::new();
        loop {
            let flat = normalize_pipes(&self.buf);
            let Some(start) = flat.find("<|DSML|tool_calls>") else {
                // 没有信封：把不是标记开头的部分发出去。
                let hold = held_tail(&flat, &["<|DSML|tool_calls>"]);
                let cut = flat.len() - hold;
                if cut == 0 {
                    return out;
                }
                let text = self.buf[..cut.min(self.buf.len())].to_string();
                self.buf.drain(..text.len());
                out.extend(self.via_thinking(vec![ScanEvent::Text(text)]));
                return out;
            };
            if start > 0 {
                let text = self.buf[..start].to_string();
                self.buf.drain(..start);
                out.extend(self.via_thinking(vec![ScanEvent::Text(text)]));
                continue;
            }
            let close = "</|DSML|tool_calls>";
            // 信封内的字节偏移与 `flat` 一致（全角替换是等长的：3 字节 → 1 字节
            // **不等长**），所以要在原串上重新定位闭合标记。
            let Some(close_at) = close_in(&self.buf, close) else {
                return out; // 信封没闭合，等后面的 chunk
            };
            let open_len = open_len_in(&self.buf);
            let body = self.buf[open_len..close_at].to_string();
            out.extend(parse_dsml_invocations(&body));
            self.buf.drain(..close_at + close.len());
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

/// 在原串里找 `</｜DSML｜tool_calls>` 或 ASCII 版本，返回字节位置。
fn close_in(s: &str, ascii: &str) -> Option<usize> {
    let full = ascii.replace('|', "\u{FF5C}");
    s.find(ascii).or_else(|| s.find(&full))
}

fn open_len_in(s: &str) -> usize {
    let full = "<\u{FF5C}DSML\u{FF5C}tool_calls>";
    if s.starts_with(full) {
        full.len()
    } else {
        "<|DSML|tool_calls>".len()
    }
}

/// 从一个 DSML 信封体里抠出所有 `invoke`。
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
        // `string="true"` 时原样；否则按 JSON 解一次（数字、布尔、对象）。
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

/// 从一个开标签里取属性值。`name="read"` / `name='read'` 两种引号都认。
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

// ---------------------------------------------------------------------------
// Qwen 的 `<tool_calls>` XML
// ---------------------------------------------------------------------------

/// Qwen 系（vLLM / DashScope 的 chat template）用一套更简的 XML：
///
/// ```text
/// <tool_calls>
///   <tool_call>
///     <function=read>
///       <parameter=path>a.rs</parameter>
///     </function>
///   </tool_call>
/// </tool_calls>
/// ```
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
                return out; // 没闭合，等下一个 chunk
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 把一段文本喂进去，返回（正文, 思考, 调用）三条。
    fn run(s: &mut dyn Scanner, text: &str) -> (String, String, Vec<(String, String)>) {
        let mut out = (String::new(), String::new(), Vec::new());
        for e in s.feed(text).into_iter().chain(s.flush()) {
            match e {
                ScanEvent::Text(t) => out.0.push_str(&t),
                ScanEvent::Thinking(t) => out.1.push_str(&t),
                ScanEvent::ToolCall { name, arguments } => out.2.push((name, arguments)),
            }
        }
        out
    }

    /// 切开喂和一次喂，结果必须一样。
    ///
    /// 这是这一层最核心的不变量：标记会跨 chunk 裂开，扣留逻辑错了就会漏。
    fn split_equals_whole(mk: fn() -> Box<dyn Scanner>, text: &str) {
        let whole = run(&mut *mk(), text);
        for step in [1, 2, 3, 5, 7, 13] {
            let mut s = mk();
            let mut acc = (String::new(), String::new(), Vec::new());
            let bytes = text.as_bytes();
            let mut i = 0;
            while i < bytes.len() {
                let mut end = (i + step).min(bytes.len());
                while end < bytes.len() && !text.is_char_boundary(end) {
                    end += 1;
                }
                for e in s.feed(&text[i..end]) {
                    match e {
                        ScanEvent::Text(t) => acc.0.push_str(&t),
                        ScanEvent::Thinking(t) => acc.1.push_str(&t),
                        ScanEvent::ToolCall { name, arguments } => acc.2.push((name, arguments)),
                    }
                }
                i = end;
            }
            for e in s.flush() {
                match e {
                    ScanEvent::Text(t) => acc.0.push_str(&t),
                    ScanEvent::Thinking(t) => acc.1.push_str(&t),
                    ScanEvent::ToolCall { name, arguments } => acc.2.push((name, arguments)),
                }
            }
            assert_eq!(acc, whole, "每 {step} 字节切一次的结果和整体喂不一样");
        }
    }

    #[test]
    fn thinking_fences_become_thinking_events() {
        let text = "答案在这。<think>让我想想……</think>就是它。";
        let got = run(&mut ThinkingScanner::new(), text);
        assert_eq!(got.0, "答案在这。就是它。");
        assert_eq!(got.1, "让我想想……");
        split_equals_whole(|| Box::new(ThinkingScanner::new()), text);
    }

    /// 宿主把开标记预填进 prompt（DeepSeek-R1 / Qwen3-Thinking 都这样），
    /// 模型只吐闭合标记——那它前面吐的**全是思考**。
    #[test]
    fn every_thinking_fence_shape_is_recognized() {
        for (open, close) in THINK_FENCES {
            let text = format!("前{open}中间{close}后");
            let got = run(&mut ThinkingScanner::new(), &text);
            assert_eq!(got.0, "前后", "围栏 {open:?} 没被认出来");
            assert_eq!(got.1, "中间", "围栏 {open:?} 的内容没进思考");
            split_equals_whole(|| Box::new(ThinkingScanner::new()), &text);
        }
    }

    /// 没有围栏时一个字都不能动。
    #[test]
    fn plain_text_passes_through_untouched() {
        let text = "普通正文，带 <div> 和 a < b。";
        let got = run(&mut ThinkingScanner::new(), text);
        assert_eq!(got.0, text);
        assert_eq!(got.1, "");
        assert!(got.2.is_empty());
        split_equals_whole(|| Box::new(ThinkingScanner::new()), text);
    }

    #[test]
    fn kimi_tool_calls_become_events() {
        let text = "我看看。\n<|tool_calls_section_begin|>\
                    <|tool_call_begin|>functions.read:0<|tool_call_argument_begin|>{\"path\":\"a.rs\"}<|tool_call_end|>\
                    <|tool_calls_section_end|>";
        let got = run(&mut KimiScanner::new(), text);
        assert_eq!(got.0.trim(), "我看看。");
        assert_eq!(got.2, vec![("read".to_string(), r#"{"path":"a.rs"}"#.to_string())]);
        split_equals_whole(|| Box::new(KimiScanner::new()), text);
    }

    /// 名字里没有 `functions.` 前缀、也没有 `:index` 的写法。
    #[test]
    fn a_bare_kimi_function_name_still_works() {
        let text = "<|tool_call_begin|>bash<|tool_call_argument_begin|>{\"command\":\"ls\"}<|tool_call_end|>";
        let got = run(&mut KimiScanner::new(), text);
        assert_eq!(got.2, vec![("bash".to_string(), r#"{"command":"ls"}"#.to_string())]);
    }

    #[test]
    fn dsml_envelopes_become_events() {
        let text = "<\u{FF5C}DSML\u{FF5C}tool_calls>\
                    <\u{FF5C}DSML\u{FF5C}invoke name=\"read\">\
                    <\u{FF5C}DSML\u{FF5C}parameter name=\"path\" string=\"true\">a.rs</\u{FF5C}DSML\u{FF5C}parameter>\
                    </\u{FF5C}DSML\u{FF5C}invoke></\u{FF5C}DSML\u{FF5C}tool_calls>";
        let got = run(&mut XmlScanner::new(), text);
        assert_eq!(got.2.len(), 1, "应当认出一个调用：{got:?}");
        assert_eq!(got.2[0].0, "read");
        assert_eq!(got.2[0].1, r#"{"path":"a.rs"}"#);
        split_equals_whole(|| Box::new(XmlScanner::new()), text);
    }

    /// ASCII 竖线的拼法同样认。
    #[test]
    fn ascii_pipe_dsml_works_too() {
        let text = "<|DSML|tool_calls><|DSML|invoke name=\"bash\">\
                    <|DSML|parameter name=\"command\" string=\"true\">ls</|DSML|parameter>\
                    </|DSML|invoke></|DSML|tool_calls>";
        let got = run(&mut XmlScanner::new(), text);
        assert_eq!(got.2.len(), 1, "{got:?}");
        assert_eq!(got.2[0].0, "bash");
        assert_eq!(got.2[0].1, r#"{"command":"ls"}"#);
    }

    #[test]
    fn qwen_xml_becomes_events() {
        let text = "查一下<tool_calls><tool_call><function=read>\
                    <parameter=path>a.rs</parameter></function></tool_call></tool_calls>";
        let got = run(&mut QwenXmlScanner::new(), text);
        assert_eq!(got.0, "查一下");
        assert_eq!(got.2, vec![("read".to_string(), r#"{"path":"a.rs"}"#.to_string())]);
        split_equals_whole(|| Box::new(QwenXmlScanner::new()), text);
    }

    /// 流断了（信封没闭合）时，扣住的内容当**正文**吐出来——宁可让用户看到
    /// 一段奇怪的文本，也不要静默吞掉模型写的东西。
    #[test]
    fn an_unterminated_block_is_handed_over_as_text() {
        let text = "先这样<|tool_call_begin|>read:0<|tool_call_argument_begin|>{\"pa";
        let got = run(&mut KimiScanner::new(), text);
        assert!(got.2.is_empty(), "没闭合就不该产出调用：{got:?}");
        assert!(got.0.contains("先这样"), "正文部分要保留：{got:?}");
        assert!(
            got.0.contains("\"pa"),
            "扣住的内容也要交出来，不能吞：{got:?}"
        );
    }
}
