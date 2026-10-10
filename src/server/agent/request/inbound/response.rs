//! The response-side wire types and the streaming stitch. Per-gateway shape differences are absorbed
//! here; the internal types stay clean.

use serde::Deserialize;
use serde_json::Value;

use crate::server::agent::request::message::{StopReason, Usage};

// The wire shape of usage, kept separate from the internal `Usage`.
#[derive(Debug, Deserialize)]
pub(crate) struct ApiUsage {
    pub(crate) prompt_tokens: u64,
    pub(crate) completion_tokens: u64,
    pub(crate) total_tokens: u64,
    #[serde(default)]
    pub(crate) completion_tokens_details: Option<CompletionDetails>,
    #[serde(default)]
    pub(crate) prompt_tokens_details: Option<PromptDetails>,
    /// Cache-read count when the host puts it at the **top level**: there is no single agreed home for
    /// it, so every spelling is read. A cache column that under-reports is money.
    #[serde(default)]
    pub(crate) cached_tokens: Option<u64>,
    /// DeepSeek-family spelling of the same count.
    #[serde(default)]
    pub(crate) prompt_cache_hit_tokens: Option<u64>,
    /// The Anthropic spelling of the same number, top level (`cache_read_input_tokens`).
    #[serde(default)]
    pub(crate) cache_read_input_tokens: Option<u64>,
    /// Anthropic spelling of the cache-write count, top level
    /// (`cache_creation_input_tokens`).
    #[serde(default)]
    pub(crate) cache_creation_input_tokens: Option<u64>,
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
    /// The unadjusted number of prompt tokens written into the cache
    /// (OpenAI's spelling).
    #[serde(default)]
    pub(crate) cache_write_tokens: Option<u64>,
    /// The same number in Alibaba's OpenAI-compatible spelling
    /// (`prompt_tokens_details.cache_creation_input_tokens`).
    #[serde(default)]
    pub(crate) cache_creation_input_tokens: Option<u64>,
}

impl From<ApiUsage> for Usage {
    fn from(u: ApiUsage) -> Self {
        // First **positive** wins, not first present: an explicit `0` must not shadow a real count.
        let cached = [
            u.cached_tokens,
            u.prompt_tokens_details.as_ref().and_then(|d| d.cached_tokens),
            u.prompt_cache_hit_tokens,
            u.cache_read_input_tokens,
        ]
        .into_iter()
        .flatten()
        .find(|n| *n > 0);
        let written = [
            u.prompt_tokens_details
                .as_ref()
                .and_then(|d| d.cache_write_tokens),
            u.prompt_tokens_details
                .as_ref()
                .and_then(|d| d.cache_creation_input_tokens),
            u.cache_creation_input_tokens,
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
            cache_write_tokens: written,
        }
    }
}

// Stitches streamed tool calls: one call arrives as several fragments, paired by `index` and merged by
// concatenating `arguments`. Treating `index` as a position, or overwriting per fragment, is the bug.
#[derive(Default)]
pub(crate) struct ToolCallAccum {
    // index -> call currently being assembled
    pub(crate) pending: Vec<(u64, crate::server::agent::request::message::ToolCall)>,
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
                    // A new call: id/name usually arrive first, but not always.
                    self.pending.push((
                        index,
                        crate::server::agent::request::message::ToolCall::new(
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
                // id/name are repeated verbatim in later deltas: overwrite, never append.
                if entry.id.is_empty() || entry.id != id {
                    entry.id = id.to_string();
                }
            }
            if let Some(f) = raw.get("function") {
                if let Some(n) = f.get("name").and_then(|v| v.as_str())
                    && !n.is_empty()
                {
                    // Same for name: overwrite, never append. Only `arguments` accumulates.
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
    pub(crate) fn finish(self) -> Vec<crate::server::agent::request::message::ToolCall> {
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
