//! The compat switch types. Types only — the detection rules live in [`super::defaults`] and
//! [`super::engine`].

use serde::{Deserialize, Serialize};
/// Which field carries max tokens: OpenAI's newer models take only `max_completion_tokens`, older ones
/// and most compatible providers take only `max_tokens`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaxTokensField {
    MaxTokens,
    MaxCompletionTokens,
}

impl MaxTokensField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MaxTokens => "max_tokens",
            Self::MaxCompletionTokens => "max_completion_tokens",
        }
    }
}

/// Which convention the thinking parameters follow (`openai`, `openrouter`, `deepseek`, `together`,
/// `zai`, `qwen`, `chat-template`, `baseten`, …) — one intent, a dozen request shapes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThinkingFormat {
    Openai,
    Openrouter,
    Deepseek,
    Together,
    Baseten,
    Zai,
    Kimi,
    Qwen,
    QwenChatTemplate,
    ChatTemplate,
    StringThinking,
    AntLing,
}

/// How "do not think this round" is expressed: dropping the parameter is often not enough — the model
/// thinks anyway, or a strict host answers 400.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReasoningDisableMode {
    /// Send no reasoning parameter at all.
    Omit,
    /// Send the lowest effort instead of turning it off.
    LowestEffort,
    /// Send `effort: "none"`.
    NoneEffort,
    OpenrouterEnabledFalse,
    ClineEnabledFalse,
    VeniceDisableThinking,
    ZaiThinkingDisabled,
    QwenEnableThinkingFalse,
    QwenTemplateFalse,
    ChatTemplateThinkingFalse,
}

/// Repair markers leaked into the visible text (chat-template, tool-call and thinking tags), for hosts
/// that do not strip them server-side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkupHealing {
    /// Kimi's `◁think▷` and tool markers.
    Kimi,
    /// DeepSeek's DSML envelope.
    Dsml,
    /// Qwen's `<tool_call>` envelope.
    Qwen,
    /// The generic `<thinking>` fence.
    Thinking,
}


/// How a tool definition's `strict` field is sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStrictMode {
    /// Say nothing; each tool decides.
    Mixed,
    /// `strict: true` on every tool.
    AllStrict,
    /// None of them.
    None,
}

/// Which field of an assistant message carries the reasoning content when history is replayed: some
/// backends treat a lost reasoning field as tampering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningContentField {
    ReasoningContent,
    Reasoning,
    ReasoningText,
}

impl ReasoningContentField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReasoningContent => "reasoning_content",
            Self::Reasoning => "reasoning",
            Self::ReasoningText => "reasoning_text",
        }
    }
}

/// The spelling of an explicit cache breakpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheControlFormat {
    /// Anthropic's `cache_control: { type: "ephemeral" }` marker.
    Anthropic,
}


/// Which top-level field caps reasoning tokens: on these endpoints reasoning and the answer share
/// `max_tokens`, so an uncapped turn can emit no answer at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingTokenBudgetField {
    /// vLLM。
    ThinkingTokenBudget,
    /// Qwen / DashScope / SGLang。
    ThinkingBudget,
    /// The other school's name for it.
    ThinkingBudgetTokens,
}

impl ThinkingTokenBudgetField {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ThinkingTokenBudget => "thinking_token_budget",
            Self::ThinkingBudget => "thinking_budget",
            Self::ThinkingBudgetTokens => "thinking_budget_tokens",
        }
    }
}

/// The streaming-reasoning loop guard, per model family (some models loop on "let me think again").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLoopGuard {
    Gemini,
    Deepseek,
    Xai,
}


/// The value of `thinking.keep` (Z.ai / Moonshot family).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ThinkingKeep {
    All(ThinkingKeepAll),
    False(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingKeepAll {
    All,
}
/// The compat switch table — **the single source of truth**. `Compat` (what the user wrote, all
/// `Option`), `Resolved` (what the code reads, all settled) and the transfer are generated from it.
macro_rules! compat_table {
    // `Compat` fields are always `Option<$ty>`; only `Resolved` differs between forms.
    (@resolved_ty value $ty:ty) => { $ty };
    (@resolved_ty relay $ty:ty) => { Option<$ty> };
    // `$target` comes from the caller (`self`): a literal `self` would not resolve after expansion.
    (@apply $target:expr, value, $user:expr, $field:ident) => {
        if let Some(v) = $user.$field { $target.$field = v; }
    };
    (@apply $target:expr, relay, $user:expr, $field:ident) => {
        if let Some(v) = $user.$field.clone() { $target.$field = Some(v); }
    };

    ($( $(#[$doc:meta])* $kind:ident $field:ident : $ty:ty = $default:expr; )*) => {
        /// What the user wrote; every field `Option`, `None` = no opinion. Names match this file
        /// (camelCase), and a typo is an error (`deny_unknown_fields`).
        #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
        #[serde(default, rename_all = "camelCase", deny_unknown_fields)]
        pub struct Compat {
            $( $(#[$doc])* pub $field: Option<$ty>, )*
        }

        /// The switches after detection. The assembly layer reads these; only body-shaping fields are
        /// settled, relay fields (`extra_body`, `chat_template_kwargs`) stay `Option`.
        #[derive(Debug, Clone, PartialEq)]
        pub struct Resolved {
            $( $(#[$doc])* pub $field: compat_table!(@resolved_ty $kind $ty), )*
        }

        impl Resolved {
            /// The conservative baseline: what an unknown host gets.
            pub fn conservative() -> Self {
                Self { $( $field: $default, )* }
            }

            /// Lay the user's choices over it: only fields they spoke about are overwritten.
            pub fn apply(&mut self, user: &Compat) {
                $( compat_table!(@apply self, $kind, user, $field); )*
            }
        }
    };
}

// ---- the switch table ----

compat_table! {
    // ---------- request body shape ----------
    value supports_store: bool = false;

    /// The `developer` role is accepted instead of always using `system`.
    value supports_developer_role: bool = false;

    /// Several leading system/developer messages are accepted. When false, the ordered prompts are
    /// joined with `\n\n` into one: strict chat templates take only one.
    value supports_multiple_system_messages: bool = false;

    /// Which field carries max tokens.
    value max_tokens_field: MaxTokensField = MaxTokensField::MaxTokens;

    /// Explicit sampling parameters (`temperature`, `top_p`, …) are accepted. OpenAI's reasoning
    /// models reject them on every host (`400 Unsupported parameter: 'temperature' ...`).
    value supports_sampling_params: bool = true;

    /// Accepts presence/frequency penalties and `stop` sequences.
    value supports_penalty_and_stop_params: bool = true;

    /// The streaming response carries `usage`.
    value supports_usage_in_streaming: bool = true;

    /// The streaming response carries `finish_reason`. When false, the end of the stream decides
    /// between stop and tool calls.
    value supports_finish_reason: bool = true;

    /// **Remove** `thinking` when `reasoning_effort` is set: some hosts (Fireworks) reject both at
    /// once. Judged after the `extra_body` merge, so a hand-written field counts too.
    value drop_thinking_when_reasoning_effort: bool = false;

    /// Any reasoning parameter may be sent.
    value supports_reasoning_params: bool = true;

    // ---------- reasoning ----------
    /// `reasoning_effort` is accepted.
    value supports_reasoning_effort: bool = false;

    /// No `reasoning_effort`, even for a model that thinks natively.
    value omit_reasoning_effort: bool = false;

    /// Which convention the reasoning parameters follow.
    value thinking_format: ThinkingFormat = ThinkingFormat::Openai;

    /// Which field carries reasoning content when history is replayed.
    value reasoning_content_field: ReasoningContentField = ReasoningContentField::ReasoningContent;

    /// How thinking is explicitly turned off.
    value reasoning_disable_mode: ReasoningDisableMode = ReasoningDisableMode::Omit;

    /// Assistant tool-call messages must carry reasoning content.
    value requires_reasoning_content_for_tool_calls: bool = false;

    /// **Every** assistant message must carry reasoning content.
    value requires_reasoning_content_for_all_assistant_turns: bool = false;

    /// A placeholder (e.g. `"."`) may stand in when a tool-call turn has no reasoning content.
    value allows_synthetic_reasoning_content_for_tool_calls: bool = true;

    /// Replay reasoning verbatim on every assistant turn that had it: some backends re-run the whole
    /// template and rebuild `<think>` blocks from it, so losing it invalidates the KV cache.
    value resend_reasoning_content: bool = false;

    /// Qwen3.6+: send `preserve_thinking: true` so the template keeps `<think>` on every assistant
    /// turn, not just after the last user message.
    value qwen_preserve_thinking: bool = false;

    /// Qwen3.8+: map the requested effort onto the template's `reasoning_effort` kwarg.
    value qwen_template_reasoning_effort: bool = false;

    /// The reasoning delta may be the **whole** text so far, not an increment: check before appending.
    value reasoning_deltas_may_be_cumulative: bool = false;

    /// Kimi K3 uses Moonshot's native `reasoning_effort`.
    value native_kimi_k3_reasoning: bool = false;

    /// GLM-5.2's Z.ai `reasoning_effort` dialect.
    value zai_reasoning_effort_dialect: bool = false;

    /// The reasoning loop guard.
    relay thinking_loop_guard: ThinkingLoopGuard = None;

    /// `thinking.keep` for the Z.ai / Moonshot family.
    relay thinking_keep: ThinkingKeep = None;

    // ---------- tools and message pairing ----------
    /// Tool-result messages must carry `name`.
    value requires_tool_result_name: bool = false;

    /// An assistant message must sit between a tool result and the next user message.
    value requires_assistant_after_tool_result: bool = false;

    /// Assistant tool-call messages must have non-empty `content`.
    value requires_assistant_content_for_tool_calls: bool = false;

    /// Tool-call ids are normalised to Mistral's form (exactly 9 alphanumerics).
    value requires_mistral_tool_ids: bool = false;

    /// Tool-call ids are cut to OpenAI's 40-character limit.
    value uses_openai_tool_call_id_limit: bool = false;

    /// The `tool_choice` parameter is accepted.
    value supports_tool_choice: bool = true;

    /// A forced `tool_choice` is accepted; when false it degrades to `"auto"`.
    value supports_forced_tool_choice: bool = true;

    /// The object form of `tool_choice` (named tools) is accepted.
    value supports_named_tool_choice: bool = true;

    /// Forcing a tool call drops the reasoning fields (Kimi answers 400 otherwise).
    value disable_reasoning_on_forced_tool_choice: bool = false;

    /// Any request carrying `tool_choice` drops the reasoning fields.
    value disable_reasoning_on_tool_choice: bool = false;

    /// Any request carrying tools drops the reasoning fields.
    value disable_reasoning_with_tools: bool = false;

    /// The `strict` field of a tool definition is accepted.
    value supports_strict_mode: bool = false;

    /// `strict` goes out on all tools or none (hosts that reject a mix).
    value tool_strict_mode: ToolStrictMode = ToolStrictMode::Mixed;

    // ---------- stream ----------
    /// First-event timeout in ms; 0 = unlimited (a host may process the prompt for a long time).
    relay stream_first_event_timeout_ms: u64 = None;

    /// The inter-event idle bound in ms, slack for slow hosts.
    relay stream_idle_timeout_ms: u64 = None;

    /// Strips leaked DeepSeek template markers from the visible text.
    value strip_deepseek_special_tokens: bool = false;

    /// Repair leaked template, tool-call and thinking markers.
    relay stream_markup_healing_pattern: MarkupHealing = None;

    /// A stream that ends empty with `length` is a context-overflow error.
    value empty_length_finish_is_context_error: bool = false;

    // ---------- misc ----------
    /// Thinking goes out as a text block delimited by `<thinking>`.
    value requires_thinking_as_text: bool = false;

    /// Image inputs are stripped before encoding (text-only multimodal ids).
    value strip_image_input: bool = false;

    /// Accepts `detail: "original"` image hints (GitHub Copilot rejects them).
    value supports_image_detail_original: bool = false;

    /// Extra fields for the request body (gateway routing hints). The escape hatch for any
    /// non-standard parameter of a host not in this table.
    relay extra_body: serde_json::Map<String, serde_json::Value> = None;

    /// `chat_template_kwargs` for `thinking_format: chat-template`.
    relay chat_template_kwargs: serde_json::Map<String, serde_json::Value> = None;

    /// `chat_template_args` for `thinking_format: baseten`.
    relay chat_template_args: serde_json::Map<String, serde_json::Value> = None;

    /// Our effort levels mapped onto the host's.
    relay reasoning_effort_map: std::collections::BTreeMap<String, String> = None;

    /// The field name for the reasoning token cap.
    relay thinking_token_budget_field: ThinkingTokenBudgetField = None;

    // ---------- cache and session affinity ----------
    /// The spelling of an explicit cache breakpoint.
    relay cache_control_format: CacheControlFormat = None;

    /// Accepts OpenAI's explicit `prompt_cache_breakpoint` content marker.
    value supports_prompt_cache_breakpoints: bool = false;

    /// The minimum lifetime of a breakpoint (only `30m` today).
    relay prompt_cache_breakpoint_ttl: String = None;

    /// The host understands `prompt_cache_key`: without it the hit rate is lower — no error, no
    /// slowdown, just the bill and the latency.
    value supports_prompt_cache_key: bool = false;

    /// A request header must follow the normalised cache key.
    relay prompt_cache_session_header: String = None;

    /// Long cache retention (`prompt_cache_retention: "24h"`).
    value supports_long_prompt_cache_retention: bool = false;

    /// Switches overlaid on a request that actually thinks: pointer replacement in the assembly layer,
    /// never spread into a full struct.
    relay when_thinking: Box<WhenThinking> = None;

    // ---------- generation parameters ----------
    /// Sampling temperature; reasoning models reject it (see `supports_sampling_params`).
    relay temperature: f64 = None;

    /// Nucleus sampling.
    relay top_p: f64 = None;

    /// top-k (not on OpenAI's own, local backends take it).
    relay top_k: f64 = None;

    /// min-p (same).
    relay min_p: f64 = None;

    /// Presence penalty.
    relay presence_penalty: f64 = None;

    /// Frequency penalty.
    relay frequency_penalty: f64 = None;

    /// Stop sequences.
    relay stop: Vec<String> = None;

    /// Random seed.
    relay seed: i64 = None;

    /// The effort level; encoded per `thinking_format` by the assembly layer.
    relay reasoning_effort: String = None;

    /// `tool_choice` in string form; the object form is `raw_tool_choice`.
    relay tool_choice: String = None;

    /// The full object form of `tool_choice` (named tools).
    relay raw_tool_choice: serde_json::Value = None;

    /// `response_format`, passed through.
    relay response_format: serde_json::Value = None;

    /// User id (OpenAI attributes abuse with it).
    relay user: String = None;

    /// The reasoning token cap; `thinking_token_budget_field` names the field, this is the value. Sent
    /// only when written.
    relay thinking_budget: u64 = None;
}

/// The payload of `Compat::when_thinking`: the same shape, but it cannot nest itself. Only requests
/// that actually think overlay it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WhenThinking {
    pub supports_store: Option<bool>,
    pub supports_developer_role: Option<bool>,
    pub supports_multiple_system_messages: Option<bool>,
    pub max_tokens_field: Option<MaxTokensField>,
    pub supports_sampling_params: Option<bool>,
    pub supports_penalty_and_stop_params: Option<bool>,
    pub supports_reasoning_effort: Option<bool>,
    pub supports_reasoning_params: Option<bool>,
    pub reasoning_effort_map: Option<std::collections::BTreeMap<String, String>>,
    pub thinking_format: Option<ThinkingFormat>,
    pub reasoning_content_field: Option<ReasoningContentField>,
    pub supports_tool_choice: Option<bool>,
    pub supports_forced_tool_choice: Option<bool>,
    pub supports_strict_mode: Option<bool>,
    pub tool_strict_mode: Option<ToolStrictMode>,
    pub extra_body: Option<serde_json::Map<String, serde_json::Value>>,
}
