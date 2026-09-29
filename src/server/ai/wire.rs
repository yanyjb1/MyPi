//! 响应侧的 wire 类型与流式拼接。
//!
//! 这一层吸收各家网关的**形状差异**，内部类型（`types::Usage` 等）保持
//! 干净：宿主把缓存数放在哪里、工具调用分几片到达，都是这里的事。

use serde::Deserialize;
use serde_json::Value;

use crate::server::ai::types::{StopReason, Usage};

// Non-streaming response body. Only the fields we need are parsed; serde leniency skips the rest.
#[derive(Debug, Deserialize)]
pub(crate) struct ChatResponse {
    pub(crate) choices: Vec<Choice>,
    #[serde(default)]
    pub(crate) usage: Option<ApiUsage>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Choice {
    pub(crate) message: ApiMessage,
    pub(crate) finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ApiMessage {
    #[serde(default)]
    pub(crate) content: Option<String>,
    // Reasoning-content extension used by thinking models (Qwen et al.).
    #[serde(default)]
    pub(crate) reasoning_content: Option<String>,
    // Tool calls requested this round.
    //
    // Wire name is snake_case, our internal field is `tool_calls`, so no
    // rename needed. A hypothetical camelCase gateway would add its own
    // rename attribute.
    // **`null` 和缺失是两回事**：`#[serde(default)]` 只管缺失，显式
    // `"tool_calls": null` 仍会被当成类型错误（`invalid type: null, expected
    // a sequence`），整个回复解析失败。魔搭就这么发——纯文本回复带一个
    // `"tool_calls": null`——而失败发生在**反序列化**，用户看到的是"这次
    // 请求坏了"，看不出跟工具字段有关。
    #[serde(default, deserialize_with = "null_as_empty")]
    pub(crate) tool_calls: Vec<crate::server::ai::types::ToolCall>,
}

// 把显式的 `null` 读成空集合。
//
// 有的宿主对"这一轮没有"的表达是 `null` 而不是省略字段。`#[serde(default)]`
// 不覆盖这种情况——它只在字段**缺失**时生效。
fn null_as_empty<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(d)?.unwrap_or_default())
}

// Wire shape of usage (with nested _details), kept separate from the
// internal types::Usage: the wire layer absorbs per-gateway shape
// differences, the internal type stays clean.
#[derive(Debug, Deserialize)]
pub(crate) struct ApiUsage {
    pub(crate) prompt_tokens: u64,
    pub(crate) completion_tokens: u64,
    pub(crate) total_tokens: u64,
    #[serde(default)]
    pub(crate) completion_tokens_details: Option<CompletionDetails>,
    #[serde(default)]
    pub(crate) prompt_tokens_details: Option<PromptDetails>,
    /// Cache-read count, when the host puts it at the **top level**.
    ///
    /// There is no single agreed home for this number. Measured on one local
    /// forwarding gateway: it emits both this and the nested
    /// `prompt_tokens_details.cached_tokens`. Others emit only one. DeepSeek
    /// uses a third spelling. Reading one location means silently reporting
    /// zero cache reads on every host that picked a different one, and the
    /// statusline's cache column is money — it must not quietly under-report.
    #[serde(default)]
    pub(crate) cached_tokens: Option<u64>,
    /// DeepSeek-family spelling of the same count.
    #[serde(default)]
    pub(crate) prompt_cache_hit_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct CompletionDetails {
    #[serde(default)]
    pub(crate) reasoning_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PromptDetails {
    #[serde(default)]
    pub(crate) cached_tokens: Option<u64>,
}

impl From<ApiUsage> for Usage {
    fn from(u: ApiUsage) -> Self {
        // First **positive** wins, not first present: a host that fills the
        // field it does not use with an explicit `0` must not shadow the one
        // it does (oh-my-pi reads it the same way, for the same reason).
        let cached = [
            u.cached_tokens,
            u.prompt_tokens_details.as_ref().and_then(|d| d.cached_tokens),
            u.prompt_cache_hit_tokens,
        ]
        .into_iter()
        .flatten()
        .find(|n| *n > 0);
        Usage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
            reasoning_tokens: u.completion_tokens_details.and_then(|d| d.reasoning_tokens),
            cached_tokens: cached,
        }
    }
}

// Accumulator for streamed tool calls.
//
// The most counter-intuitive part of the streaming protocol: **a single
// tool call arrives as several fragments**. `index` is the pairing key,
// not an array position:
//
// ```text
// {"index":0,"id":"call_abc","function":{"name":"read_file","arguments":""}}
// {"index":0,"function":{"arguments":"{\"pa"}}
// {"index":0,"function":{"arguments":"th\":\"a\"}"}}
// ```
//
// Merge by `index`, concatenating `arguments` in order.
// Two classic bugs, both subtle:
// - Treating `index` as an array position and pushing: fine with one
//   call, chaos with parallel calls.
// - Overwriting the whole entry per fragment: only half a JSON survives,
//   and the failure looks like the model sent bad arguments.
//
// A standalone structure so the stitching logic is testable offline.
#[derive(Default)]
pub(crate) struct ToolCallAccum {
    // index -> call currently being assembled
    pub(crate) pending: Vec<(u64, crate::server::ai::types::ToolCall)>,
}

impl ToolCallAccum {
    // Feed one delta. A missing `index` is treated as 0 (single-call leniency).
    pub(crate) fn feed(&mut self, delta: &Value) {
        let Some(calls) = delta.get("tool_calls").and_then(|v| v.as_array()) else {
            return;
        };
        for raw in calls {
            let index = raw.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
            let entry = match self.pending.iter_mut().find(|(i, _)| *i == index) {
                Some((_, call)) => call,
                None => {
                    // A new call. id/name usually arrive in the first
                    // fragment, but do not assume it — some gateways put
                    // the id in the second one.
                    self.pending.push((
                        index,
                        crate::server::ai::types::ToolCall::new(
                            String::new(),
                            String::new(),
                            String::new(),
                        ),
                    ));
                    &mut self.pending.last_mut().expect("just pushed").1
                }
            };
            if let Some(id) = raw.get("id").and_then(|v| v.as_str())
                && !id.is_empty()
            {
                // Gateways often **repeat id/name verbatim** in
                // successive deltas; overwrite rather than append, or the
                // id becomes call_xxxcall_xxx and the name
                // editeditedit... Genuinely split ids are rare, and
                // append semantics belong to arguments alone.
                if entry.id.is_empty() || entry.id != id {
                    entry.id = id.to_string();
                }
            }
            if let Some(f) = raw.get("function") {
                if let Some(n) = f.get("name").and_then(|v| v.as_str())
                    && !n.is_empty()
                {
                    // Same for name: overwrite, never append (observed
                    // "editedit..." x100 in live e2e). Append semantics
                    // belong to arguments only.
                    if entry.function.name.is_empty() || entry.function.name != n {
                        entry.function.name = n.to_string();
                    }
                }
                // Arguments **concatenate**, they do not replace. Overwriting here is the classic bug.
                if let Some(a) = f.get("arguments").and_then(|v| v.as_str()) {
                    entry.function.arguments.push_str(a);
                }
            }
        }
    }

    // Finish: return calls ordered by index — preserving the order the
    // model gave.
    pub(crate) fn finish(self) -> Vec<crate::server::ai::types::ToolCall> {
        let mut v = self.pending;
        v.sort_by_key(|(i, _)| *i);
        v.into_iter().map(|(_, c)| c).collect()
    }
}

pub(crate) fn parse_finish_reason(reason: Option<String>) -> StopReason {
    match reason.as_deref() {
        Some("length") => StopReason::Length,
        // The model requested tools — the loop's continue signal
        Some("tool_calls") | Some("function_call") => StopReason::ToolCalls,
        // "stop" is the norm; unknown values are leniently treated as stop (a common compatibility-layer flaw)
        _ => StopReason::Stop,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `"tool_calls": null` 必须读成"没有调用"，不是解析失败。
    ///
    /// 魔搭的纯文本回复就长这样。失败发生在反序列化，报出来的是
    /// `invalid type: null, expected a sequence`——完全看不出跟工具有关。
    #[test]
    fn an_explicit_null_tool_calls_is_an_empty_list() {
        let body = r#"{
            "choices": [{
                "message": { "role": "assistant", "content": "包名是 mypi", "tool_calls": null },
                "finish_reason": "stop"
            }],
            "usage": null
        }"#;
        let resp: ChatResponse = serde_json::from_str(body).expect("显式 null 不该让整个回复失败");
        assert!(resp.choices[0].message.tool_calls.is_empty());
        assert_eq!(resp.choices[0].message.content.as_deref(), Some("包名是 mypi"));
        assert!(resp.usage.is_none());

        // 字段整个缺失（另一种"没有"）同样要能过。
        let body = r#"{"choices":[{"message":{"role":"assistant","content":"hi"}}]}"#;
        let resp: ChatResponse = serde_json::from_str(body).expect("缺失的 tool_calls");
        assert!(resp.choices[0].message.tool_calls.is_empty());

        // 真有调用时照常解析。
        let body = r#"{"choices":[{"message":{"role":"assistant","content":null,
            "tool_calls":[{"id":"call_1","type":"function",
                           "function":{"name":"read","arguments":"{}"}}]}}]}"#;
        let resp: ChatResponse = serde_json::from_str(body).expect("正常调用");
        assert_eq!(resp.choices[0].message.tool_calls.len(), 1);
        assert_eq!(resp.choices[0].message.tool_calls[0].function.name, "read");
    }

    #[test]
    fn finish_reason_maps_length_only() {
        assert_eq!(
            parse_finish_reason(Some("length".into())),
            StopReason::Length
        );
        assert_eq!(
            parse_finish_reason(Some("tool_calls".into())),
            StopReason::ToolCalls
        );
        assert_eq!(parse_finish_reason(Some("stop".into())), StopReason::Stop);
        // Unknown values leniently map to stop (a common compatibility-layer flaw)
        assert_eq!(parse_finish_reason(Some("weird".into())), StopReason::Stop);
        assert_eq!(parse_finish_reason(None), StopReason::Stop);
    }

    #[test]
    fn fragmented_tool_call_delta_is_stitched_back_together() {
        // The easiest place to get streaming tool calls wrong; fragments
        // captured from a real gateway: id+name in the first, arguments
        // split across three.
        let mut acc = ToolCallAccum::default();
        // Arguments split in three and stitched back to {"path":"a"} — verifies concat, not replace
        let pieces = [
            r#"{"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"read_file","arguments":""}}]}"#,
            r#"{"tool_calls":[{"index":0,"function":{"arguments":"{\"pa"}}]}"#,
            r#"{"tool_calls":[{"index":0,"function":{"arguments":"th\":\"a\"}"}}]}"#,
        ];
        for piece in pieces {
            let v: Value = serde_json::from_str(piece).unwrap();
            acc.feed(&v);
        }
        let calls = acc.finish();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call_abc");
        assert_eq!(calls[0].name(), "read_file");
        assert_eq!(calls[0].function.arguments, r#"{"path":"a"}"#);
    }

    #[test]
    fn parallel_tool_calls_keep_their_order_by_index() {
        // Two parallel calls interleaving. Index-ordered merge must
        // restore order 0, 1 — reversing it would pair results to the
        // wrong tool_call_id downstream.
        let mut acc = ToolCallAccum::default();
        for piece in [
            r#"{"tool_calls":[{"index":1,"id":"call_b","function":{"name":"write_file","arguments":""}}]}"#,
            r#"{"tool_calls":[{"index":0,"id":"call_a","function":{"name":"read_file","arguments":""}}]}"#,
            r#"{"tool_calls":[{"index":1,"function":{"arguments":"{}"}}]}"#,
            r#"{"tool_calls":[{"index":0,"function":{"arguments":"{}"}}]}"#,
        ] {
            let v: Value = serde_json::from_str(piece).unwrap();
            acc.feed(&v);
        }
        let calls = acc.finish();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name(), "read_file");
        assert_eq!(calls[1].name(), "write_file");
    }

    #[test]
    fn missing_index_defaults_to_zero() {
        // Some compatibility layers omit index. Treating it as 0 is the
        // most lenient choice; an error would only confuse users, and
        // leniency cannot misbehave in the single-call case.
        let mut acc = ToolCallAccum::default();
        let v: Value = serde_json::from_str(
            r#"{"tool_calls":[{"id":"c1","function":{"name":"f","arguments":"{}"}}]}"#,
        )
        .unwrap();
        acc.feed(&v);
        assert_eq!(acc.finish().len(), 1);
    }

    #[test]
    fn non_tool_deltas_are_ignored_by_the_accumulator() {
        // Content/reasoning chunks carry no tool_calls; feeding them must be a no-op
        let mut acc = ToolCallAccum::default();
        for piece in [
            r#"{"content":"hi"}"#,
            r#"{"reasoning_content":"hmm"}"#,
            r#"{}"#,
        ] {
            let v: Value = serde_json::from_str(piece).unwrap();
            acc.feed(&v);
        }
        assert!(acc.finish().is_empty());
    }

    #[test]
    fn a_chunk_with_empty_content_still_feeds_the_accumulator() {
        // The exact shape that killed the Neko session: Qwen via several
        // gateways streams tool calls in chunks that ALSO carry
        // `content: ""`. The wire delta itself is fine; the bug was in the
        // stream loop, which skipped this feed when the content filter
        // produced nothing. Pinned here at the accumulator contract level:
        // such a chunk MUST contribute its fragments when fed.
        let mut acc = ToolCallAccum::default();
        let pieces = [
            // One chunk: empty content AND the call's opening fragment.
            r#"{"content":"","tool_calls":[{"index":0,"id":"call_72c3f22a636645c8892be64d","type":"function","function":{"name":"bash","arguments":""}}]}"#,
            r#"{"content":"","tool_calls":[{"index":0,"function":{"arguments":"{\"command\": \"pwd && ls\"}"}}]}"#,
        ];
        for piece in pieces {
            let v: Value = serde_json::from_str(piece).unwrap();
            acc.feed(&v);
        }
        let calls = acc.finish();
        assert_eq!(calls.len(), 1, "empty-content chunk must not drop the call");
        assert_eq!(calls[0].name(), "bash");
        assert_eq!(calls[0].function.arguments, r#"{"command": "pwd && ls"}"#);
    }
}
