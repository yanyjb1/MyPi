//! 兼容开关的类型定义。
//!
//! 这里**只放类型**，不放任何判断——探测规则在 [`super::defaults`] 与
//! [`super::engine`]。字段的来源与整体设计见 [`super`]。

use serde::{Deserialize, Serialize};
/// max tokens 用哪个字段名。
///
/// OpenAI 官方的新模型只认 `max_completion_tokens`（发 `max_tokens` 会
/// 400）；老模型和绝大多数兼容服务商只认 `max_tokens`。
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

/// 推理（thinking）参数用哪一套约定。
///
/// 同一个"让模型多想一会儿"的诉求，各家在请求体里长得完全不一样：
///
/// | 值 | 请求体形状 |
/// |---|---|
/// | `openai` | `reasoning_effort: "high"` |
/// | `openrouter` | `reasoning: { effort: "high" }` |
/// | `deepseek` | `thinking: { type: "enabled" }` |
/// | `together` | `reasoning: { enabled: true }` |
/// | `zai` / `kimi` | `thinking: { type, effort }` |
/// | `qwen` | 顶层 `enable_thinking: true` |
/// | `qwen-chat-template` | `chat_template_kwargs.enable_thinking` |
/// | `chat-template` | 由 `chat_template_kwargs` 自由指定 |
/// | `string-thinking` | 顶层 `thinking: "high"`（字符串而非对象） |
/// | `baseten` | `chat_template_args` |
/// | `ant-ling` | `reasoning: { effort }`，effort 为空则整个不发 |
///
/// `deepseek` / `together` / `baseten` / `string-thinking` / `ant-ling`
/// 是 `pi` 独有，`oh-my-pi` 用 `kimi` 等合并处理。
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

/// 明确"这一轮不要推理"时，怎么表达。
///
/// 有推理能力的模型常常不能简单地把参数删掉——删了它照旧推理，或者
/// 直接 400。得按各家的约定显式关掉。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReasoningDisableMode {
    /// 只是不发推理参数。
    Omit,
    /// 发最低档的 effort，而不是关掉。
    LowestEffort,
    /// 发 `effort: "none"`。
    NoneEffort,
    OpenrouterEnabledFalse,
    ClineEnabledFalse,
    VeniceDisableThinking,
    ZaiThinkingDisabled,
    QwenEnableThinkingFalse,
    QwenTemplateFalse,
    ChatTemplateThinkingFalse,
}

/// 从可见文本里修补泄漏出来的标记（chat template / 工具调用 / 思考标签）。
///
/// 某些服务商不剥模板标记，模型吐出来的 `<|tool_calls_begin|>`、
/// `<｜DSML｜tool_calls｜>` 会原样出现在回复正文里。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkupHealing {
    /// Kimi 的 `◁think▷` / 工具标记。
    Kimi,
    /// DeepSeek 的 DSML 信封。
    Dsml,
    /// Qwen 的 `<tool_call>` 信封。
    Qwen,
    /// 通用 `<thinking>` 围栏。
    Thinking,
}


/// 工具定义的 `strict` 字段怎么发。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolStrictMode {
    /// 不表态，逐个工具按需发。
    Mixed,
    /// 全部发 `strict: true`。
    AllStrict,
    /// 全部不发。
    None,
}

/// 推理内容放在 assistant 消息的哪个字段上。
///
/// 回传历史时用：上一轮模型的思考要跟着 assistant 消息一起发回去，
/// 不然有的后端会认为历史被篡改。
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

/// 显式缓存断点的格式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheControlFormat {
    /// Anthropic 风格的 `cache_control: { type: "ephemeral" }` 标记，
    /// 打在 system 提示、最后一个工具定义、最后一条消息上。
    Anthropic,
}


/// 推理 token 上限用哪个顶层字段名。
///
/// 这些端点上推理和回答**共享** `max_tokens`，不单独限一下推理，一轮
/// 思考就能把额度吃光、一个字的回答都没有。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingTokenBudgetField {
    /// vLLM。
    ThinkingTokenBudget,
    /// Qwen / DashScope / SGLang。
    ThinkingBudget,
    /// llama.cpp。
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

/// 流式推理的循环守护，按模型家族选。
///
/// 有的推理模型会陷进"我再想想、我再想想"的循环，无限吐思考内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLoopGuard {
    Gemini,
    Deepseek,
    Xai,
}


/// `thinking.keep` 的取值（Z.ai / Moonshot 系）。
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
/// 兼容开关表——**这张表是唯一的真相**。
///
/// `Compat`（用户写的，全 `Option`）、`Resolved`（代码读的，全确定值）、
/// 以及两者之间的搬运代码，都由下面这一次声明生成。
///
/// # 为什么要生成
///
/// 手工维护两个结构体加一段搬运代码时，"加字段忘了搬"是一种**不会报错、
/// 不会变慢、什么都不发生**的失败：用户在 `models.yml` 里写了那个开关，
/// YAML 合法、启动不报错、一个字节都不影响请求。真实发生过——一次抄表里
/// 19 个字段是这种状态（16 个连 `Resolved` 都没进，3 个只在测试的
/// `assert!` 里出现过）。
///
/// 生成之后，加字段只有这一处要改。
///
/// # 两种形态
///
/// * `value`——`Compat: Option<T>` / `Resolved: T`。开关、枚举、标量。
///   搬运 = 用户写了就覆盖。
/// * `relay`——`Compat: Option<T>` / `Resolved: Option<T>`。"有就发、没有
///   就不发"的转发字段，以及生成参数。
///
/// # `=` 后面
///
/// `Resolved` 的**保守基线**：认不出来的宿主拿到的答案。选值的原则是
/// "不会惹恼严格网关"——不发一个它可能不认的字段，比省一次往返重要。

/// 兼容开关表——**这张表是唯一的真相**。
///
/// `Compat`（用户写的，全 `Option`）、`Resolved`（代码读的，全确定值）、
/// 以及两者之间的搬运代码，都由下面这一次声明生成。
///
/// # 为什么要生成
///
/// 手工维护两个结构体加一段搬运代码时，"加字段忘了搬"是一种**不会报错、
/// 不会变慢、什么都不发生**的失败：用户在 `models.yml` 里写了那个开关，
/// YAML 合法、启动不报错、一个字节都不影响请求。真实发生过——一次抄表里
/// 19 个字段是这种状态（16 个连 `Resolved` 都没进，3 个只在测试的
/// `assert!` 里出现过）。
///
/// 生成之后，加字段只有这一处要改。
///
/// # 两种形态
///
/// * `value`——`Compat: Option<T>` / `Resolved: T`。开关、枚举、标量。
///   搬运 = 用户写了就覆盖。
/// * `relay`——`Compat: Option<T>` / `Resolved: Option<T>`。"有就发、没有
///   就不发"的转发字段，以及生成参数。
///
/// # `=` 后面
///
/// `Resolved` 的**保守基线**：认不出来的宿主拿到的答案。选值的原则是
/// "不会惹恼严格网关"——不发一个它可能不认的字段，比省一次往返重要。
macro_rules! compat_table {
    // `Compat` 的字段**永远**是 `Option<$ty>`；两种形态的差别只在
    // `Resolved` 那边，以及搬运时要不要再包一层。
    (@resolved_ty value $ty:ty) => { $ty };
    (@resolved_ty relay $ty:ty) => { Option<$ty> };
    // `$target` 由调用处传入（`self`）。宏里写死的 `self` 过不了 hygiene：
    // 宏定义在 item 位置，展开后那个 `self` 解析不到方法体里的 `self`。
    (@apply $target:expr, value, $user:expr, $field:ident) => {
        if let Some(v) = $user.$field { $target.$field = v; }
    };
    (@apply $target:expr, relay, $user:expr, $field:ident) => {
        if let Some(v) = $user.$field.clone() { $target.$field = Some(v); }
    };

    ($( $(#[$doc:meta])* $kind:ident $field:ident : $ty:ty = $default:expr; )*) => {
        /// 用户的表态。全字段 `Option`：`None` = 没意见。
        ///
        /// 写进 `models.yml` 的 `compat:` 块或 `compat-rules.yaml` 的 `set:`，
        /// 字段名与本文件的 Rust 名逐字对应（camelCase）。**拼错会报错**
        /// （`deny_unknown_fields`）——一个拼错的开关等于一个不生效的开关，
        /// 而它看起来完全正常。
        #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
        #[serde(default, rename_all = "camelCase", deny_unknown_fields)]
        pub struct Compat {
            $( $(#[$doc])* pub $field: Option<$ty>, )*
        }

        /// 探测并补全后的开关。装配层直接读这里，不再做 `Option` 判断。
        ///
        /// 只有会**改变请求体形状**的字段展开成确定值；转发型字段
        /// （`extra_body`、`chat_template_kwargs`、各路由偏好）保持 `Option`
        /// 原样，它们的语义就是"有就发、没有就不发"。
        #[derive(Debug, Clone, PartialEq)]
        pub struct Resolved {
            $( $(#[$doc])* pub $field: compat_table!(@resolved_ty $kind $ty), )*
        }

        impl Resolved {
            /// 保守基线：认不出来的宿主拿到的答案。
            pub fn conservative() -> Self {
                Self { $( $field: $default, )* }
            }

            /// 把用户写的逐项叠上去。只覆盖**表过态**的字段。
            pub fn apply(&mut self, user: &Compat) {
                $( compat_table!(@apply self, $kind, user, $field); )*
            }
        }
    };
}

// ---------------------------------------------------------------------------
// 开关表
// ---------------------------------------------------------------------------

compat_table! {
    // ---------- 请求体形状 ----------
    /// 支持 `store` 字段。默认按 base_url 探测。
    value supports_store: bool = false;

    /// 支持 `developer` 角色（而不是全用 `system`）。
    value supports_developer_role: bool = false;

    /// 支持**多条**前导 system/developer 消息。
    ///
    /// false 时把有序的多段提示词用 `\n\n` 拼成一条——严格 chat 模板
    /// （vLLM 跑的 Qwen、MiniMax）只认一条。true 保留分块，改尾部时
    /// KV 缓存命中更好。
    value supports_multiple_system_messages: bool = false;

    /// max tokens 用哪个字段名。
    value max_tokens_field: MaxTokensField = MaxTokensField::MaxTokens;

    /// 调用方没给 max tokens 时也硬发一个。
    ///
    /// 接受显式采样参数（`temperature` / `top_p` / `top_k` / `min_p` /
    /// 惩罚项）。
    ///
    /// OpenAI 的推理模型（o 系、gpt-5+）在**所有**托管方都拒绝这些字段
    /// （`400 Unsupported parameter: 'temperature' is not supported with
    /// this model`）。
    value supports_sampling_params: bool = true;

    /// 接受 presence/frequency 惩罚与 `stop` 序列。
    value supports_penalty_and_stop_params: bool = true;

    /// 流式响应带 `usage`（`stream_options: { include_usage: true }` 的
    /// 约定）。默认 true。
    value supports_usage_in_streaming: bool = true;

    /// pi: 流式响应带 `finish_reason`。
    ///
    /// false 时由流的结束方式推断是 stop 还是 tool_calls。
    value supports_finish_reason: bool = true;

    /// 有 `reasoning_effort` 时**删掉** `thinking`。
    ///
    /// Fireworks 一类的宿主拒绝同时收到这两者（400）。判断放在
    /// `extra_body` 合并**之后**——用户手写进逃生舱的字段同样算数。
    value drop_thinking_when_reasoning_effort: bool = false;

    /// 允许发任何推理参数。默认按 base_url 探测（GitHub Copilot 的
    /// chat/completions 关掉）。
    value supports_reasoning_params: bool = true;

    // ---------- 推理 ----------
    /// 支持 `reasoning_effort`。
    value supports_reasoning_effort: bool = false;

    /// 即便模型原生会推理，也不发 `reasoning_effort`。
    value omit_reasoning_effort: bool = false;

    /// 推理参数用哪套约定。
    value thinking_format: ThinkingFormat = ThinkingFormat::Openai;

    /// 回传历史时推理内容放在哪个字段。
    value reasoning_content_field: ReasoningContentField = ReasoningContentField::ReasoningContent;

    /// 显式关掉推理时的表达方式。
    value reasoning_disable_mode: ReasoningDisableMode = ReasoningDisableMode::Omit;

    /// assistant 的工具调用消息必须带推理内容。
    value requires_reasoning_content_for_tool_calls: bool = false;

    /// **所有** assistant 消息都必须带推理内容。
    value requires_reasoning_content_for_all_assistant_turns: bool = false;

    /// 工具调用轮缺推理内容时，允许塞个占位符（比如 `"."`）。
    ///
    /// DeepSeek 会校验字段的真实性，对它要设 false。
    value allows_synthetic_reasoning_content_for_tool_calls: bool = true;

    /// 每个带过推理的 assistant 轮都把推理内容原样回传。
    ///
    /// 本地 llama.cpp 系后端（llama.cpp / LM Studio / vLLM / sglang /
    /// Ollama）每轮都重新 tokenize 整个 chat template，Qwen3 / DeepSeek-R1
    /// / GLM 的模板靠 `reasoning_content` 重建 `<think>` 块。丢掉这个字段
    /// 会让 assistant 轮渲染出不一样的 token 序列，KV 缓存整段失效、提示词
    /// 全部重算。
    value replay_reasoning_content: bool = false;

    /// Qwen3.6+：发 `preserve_thinking: true`，让模板为**每个** assistant
    /// 轮渲染 `<think>` 标记（而不只是最后一条用户消息之后的那些）。
    ///
    /// 不发的话，新的用户消息一到，之前的 assistant 轮就变成"旧的"、模板
    /// 会把 think 块剥掉，和缓存时的 token 对不上，SWA 模型上就是整段重算。
    value qwen_preserve_thinking: bool = false;

    /// Qwen3.8+：把要的 effort 映射到模板的 `reasoning_effort` kwarg。
    ///
    /// 不发的话 qwen 系只能开关 `enable_thinking`，模板永远按自己的
    /// `xhigh` 默认档思考。
    value qwen_template_reasoning_effort: bool = false;

    /// 同一字段的流式推理增量可能是**累积全文**而不是增量。
    /// 直接拼接会得到重复文本，要先判断。
    value reasoning_deltas_may_be_cumulative: bool = false;

    /// Kimi K3 走 Moonshot 原生 `reasoning_effort`。
    value native_kimi_k3_reasoning: bool = false;

    /// GLM-5.2 的 Z.ai `reasoning_effort` 方言。
    value zai_reasoning_effort_dialect: bool = false;

    /// 推理循环守护。
    relay thinking_loop_guard: ThinkingLoopGuard = None;

    /// Z.ai / Moonshot 系的 `thinking.keep`。
    relay thinking_keep: ThinkingKeep = None;

    // ---------- 工具与消息配对 ----------
    /// 工具结果消息必须带 `name` 字段。
    value requires_tool_result_name: bool = false;

    /// 工具结果之后的用户消息，中间必须夹一条 assistant。
    value requires_assistant_after_tool_result: bool = false;

    /// assistant 的工具调用消息必须有非空 `content`。
    value requires_assistant_content_for_tool_calls: bool = false;

    /// 工具调用 id 规范化成 Mistral 格式（恰好 9 位字母数字）。
    value requires_mistral_tool_ids: bool = false;

    /// 工具调用 id 截到 OpenAI 的 40 字符上限。
    value uses_openai_tool_call_id_limit: bool = false;

    /// 支持 `tool_choice` 参数。
    value supports_tool_choice: bool = true;

    /// 支持**强制**的 `tool_choice`（`"required"` 或具名工具）。
    ///
    /// false 时保留工具可用，但把强制降级成 `"auto"`。
    value supports_forced_tool_choice: bool = true;

    /// 支持具名工具的**对象**形式
    /// （`{ type: "function", function: { name } }`）。
    ///
    /// llama.cpp 一类只认字符串形式的 `tool_choice`。
    value supports_named_tool_choice: bool = true;

    /// 强制工具调用时，把推理字段整个去掉。
    ///
    /// Kimi 在 `tool_choice` 强制 + thinking 打开时直接 400
    /// （`tool_choice 'specified' is incompatible with thinking enabled`）。
    value disable_reasoning_on_forced_tool_choice: bool = false;

    /// **任何**带 `tool_choice` 的请求都去掉推理字段（DeepSeek 推理模型）。
    value disable_reasoning_on_tool_choice: bool = false;

    /// 只要请求带了工具就关掉推理。
    value disable_reasoning_with_tools: bool = false;

    /// 支持工具定义里的 `strict` 字段。
    value supports_strict_mode: bool = false;

    /// 工具 `strict` 是全部发还是全不发（不接受混着来的服务商用）。
    value tool_strict_mode: ToolStrictMode = ToolStrictMode::Mixed;

    // ---------- 流 ----------
    /// 首事件超时（毫秒）。`0` = 不限，允许无上限的提示词处理。
    relay stream_first_event_timeout_ms: u64 = None;

    relay stream_idle_timeout_ms: u64 = None;

    /// 事件间空闲超时的下限（毫秒），给慢速推理宿主留余量。
    /// 从可见文本里剥掉泄漏的 DeepSeek 模板标记。
    value strip_deepseek_special_tokens: bool = false;

    /// 修补泄漏的模板/工具调用/思考标记。
    relay stream_markup_healing_pattern: MarkupHealing = None;

    /// 把"空的、以 length 收尾"的流当成上下文超限错误。
    value empty_length_finish_is_context_error: bool = false;

    // ---------- 杂项 ----------
    /// 思考块必须转成带 `<thinking>` 定界的文本块发出去。
    value requires_thinking_as_text: bool = false;

    /// 编码前把图片输入剥掉（多模态 id 按纯文本服务时）。
    value strip_image_input: bool = false;

    /// 接受 `detail: "original"` 的图片提示（GitHub Copilot 拒绝）。
    value supports_image_detail_original: bool = false;

    /// 额外塞进请求体的字段（网关路由提示一类）。
    ///
    /// 这是逃生舱：表里没有的服务商，靠它发任何非标准参数。
    relay extra_body: serde_json::Map<String, serde_json::Value> = None;

    /// pi: `thinking_format: chat-template` 时发出去的 `chat_template_kwargs`。
    relay chat_template_kwargs: serde_json::Map<String, serde_json::Value> = None;

    /// pi: `thinking_format: baseten` 时发出去的 `chat_template_args`。
    relay chat_template_args: serde_json::Map<String, serde_json::Value> = None;

    /// pi 的 `reasoning_effort` 档位到服务商档位的映射。
    relay reasoning_effort_map: std::collections::BTreeMap<String, String> = None;

    /// pi: 推理 token 上限的字段名。
    relay thinking_token_budget_field: ThinkingTokenBudgetField = None;

    // ---------- 缓存与会话亲和 ----------
    /// 显式缓存断点的格式。
    relay cache_control_format: CacheControlFormat = None;

    /// 接受 OpenAI 的显式 `prompt_cache_breakpoint` 内容标记。
    value supports_prompt_cache_breakpoints: bool = false;

    /// 显式缓存断点的最短存活期（当前只支持 `30m`）。
    relay prompt_cache_breakpoint_ttl: String = None;

    /// 宿主认 `prompt_cache_key`（OpenAI 官方提高命中率的主要手段）。
    ///
    /// 服务端按这个键把"同一段前缀"的请求归到同一份缓存。不发的话命中率
    /// 低于本该有的水平——**不报错、不变慢，只有账单和延迟的差别**。
    value supports_prompt_cache_key: bool = false;

    /// 某个请求头要跟规范化后的缓存键保持一致。
    relay prompt_cache_session_header: String = None;

    /// 长缓存存活期（`prompt_cache_retention: "24h"`）。
    value supports_long_prompt_cache_retention: bool = false;

    /// 进入思考模式时额外叠加的开关。
    ///
    /// 只有"真的要推理"的请求（要求了推理、没被关掉、模型支持、没被
    /// 强制工具调用压掉）才叠。装配层在这里做指针替换，不展开成完整结构。
    relay when_thinking: Box<WhenThinking> = None;

    // ---------- 生成参数 ----------
    //
    // 这些不是"兼容开关"而是"这个模型我想怎么用"，但同属 per-model 配置，
    // 放一起最省事：`models.yml` 里一个模型一条记录，一个 `compat:` 块
    // 说完所有事。
    //
    // 每一项都是"不写就不发"。绝不替用户编造默认值——服务端自己的默认
    // 比客户端猜的准。
    /// 采样温度。推理模型（o 系、gpt-5+）拒绝这个字段，见
    /// [`Compat::supports_sampling_params`]。
    relay temperature: f64 = None;

    /// 核采样。
    relay top_p: f64 = None;

    /// top-k（OpenAI 官方不收，本地后端收）。
    relay top_k: f64 = None;

    /// min-p（同上）。
    relay min_p: f64 = None;

    /// 存在惩罚。
    relay presence_penalty: f64 = None;

    /// 频率惩罚。
    relay frequency_penalty: f64 = None;

    /// 停止序列。
    relay stop: Vec<String> = None;

    /// 随机种子。
    relay seed: i64 = None;

    /// 推理档位（`low` / `medium` / `high` / `xhigh` …）。
    /// 按 [`Compat::thinking_format`] 编成各家形状发出去。
    relay reasoning_effort: String = None;

    /// `tool_choice`：`"auto"` / `"none"` / `"required"`，或具名对象。
    /// 字符串形式在这里写；具名对象用 `raw_tool_choice`。
    relay tool_choice: String = None;

    /// `tool_choice` 的完整对象形式（具名工具用）。
    relay raw_tool_choice: serde_json::Value = None;

    /// `response_format` 原样透传（`{ type: "json_object" }` 一类）。
    relay response_format: serde_json::Value = None;

    /// 用户标识（OpenAI 用来归因滥用）。
    relay user: String = None;

    /// 推理 token 上限。和 [`Compat::thinking_token_budget_field`] 配套：
    /// 那个决定字段名，这个决定值。写了才发。
    ///
    /// Reasoning and the answer share `max_tokens` on the backends that have
    /// this field, so without a cap a reasoning-heavy turn can consume the
    /// whole budget and emit no answer.
    relay thinking_budget: u64 = None;
}

/// [`Compat::when_thinking`] 的载荷：与 [`Compat`] 同形，但不能嵌套自己。
///
/// 只有"真的要推理"的请求（要求了推理、没被关掉、模型支持、没被强制工具
/// 调用压掉）才叠。装配层在这里做指针替换，不展开成完整结构。
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
