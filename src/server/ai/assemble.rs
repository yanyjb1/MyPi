//! 请求体装配。
//!
//! 唯一的职责：把 `(model, max_tokens, stream, messages, tools, compat)`
//! 变成一个 JSON 对象。**无 IO、无全局状态、不读本地配置**——这三条是
//! `replay_round` 能只靠数据库重建请求体的前提，也是它能被单元测试直接
//! 调用的原因。

/// Serialize one request body.
///
/// **The single source of the request's bytes.** The live path (`complete` /
/// `stream`) and the replay path (rebuilding a stored round from the DB) both
/// call this, so a replayed request cannot drift from the one that was sent —
/// which is the whole point of storing the header at all. Everything variable
/// arrives as an argument: nothing is read from local config here.
///
/// # Why a `Map` and not a struct
///
/// The field *names* are decided at runtime: `max_tokens` or
/// `max_completion_tokens`, twelve different encodings for the thinking
/// parameter, four downgrade paths for `tool_choice`. A serde struct can only
/// describe one fixed shape, so every branch would have to be re-inserted into
/// the serialized `Value` afterwards — strictly harder to read than building
/// the map directly. `serde_json::Map` is a `BTreeMap`, so keys come out in a
/// stable order: the same inputs always produce the same bytes, and
/// byte-exact replay is unaffected.
///
/// # Where the switches come from
///
/// `compat` is the output of [`crate::server::ai::compat::resolve`] — detection
/// plus whatever the user wrote under `compat:` in `models.yml`. **Every
/// decision reads it**; this function does no detection of its own.
pub(crate) fn body_json(
    model: &str,
    max_tokens: u32,
    stream: bool,
    messages: &serde_json::Value,
    tools: &serde_json::Value,
    compat: &crate::server::ai::compat::Resolved,
    session_id: Option<&str>,
) -> serde_json::Value {
    use crate::server::ai::compat::{ReasoningDisableMode, ThinkingFormat};
    use serde_json::{json, Map, Value};

    fn put_num(body: &mut Map<String, Value>, key: &str, v: Option<f64>) {
        if let Some(x) = v {
            body.insert(key.to_string(), json!(x));
        }
    }

    let mut body: Map<String, Value> = Map::new();
    body.insert("model".to_string(), Value::String(model.to_string()));

    // OpenAI 的 `store: false`：别把这段对话留下来。语义是"不同意被拿去
    // 训练"，只在认这个字段的宿主上发。
    if compat.supports_store {
        body.insert("store".to_string(), Value::Bool(false));
    }

    // `prompt_cache_key`：服务端按这个键把"同一段前缀"的请求归到同一份
    // 缓存里。不发的话命中率低于本该有的水平——**不报错、不变慢，只有
    // 账单和延迟的差别**。值就是会话 id，同一个会话的每一轮都一样。
    if compat.supports_prompt_cache_key
        && let Some(sid) = session_id
    {
        body.insert(
            "prompt_cache_key".to_string(),
            Value::String(normalize_cache_key(sid)),
        );
    }
    body.insert("messages".to_string(), messages.clone());
    body.insert("stream".to_string(), Value::Bool(stream));

    // ---- Is this request thinking? Three gates, in order of authority. ----
    //
    // 1. the caller asked for a level (`reasoning_effort` set),
    // 2. this wire may carry reasoning params at all,
    // 3. no tool-choice rule forbids it.
    //
    // (3) is a **provider constraint**, not a preference: Kimi answers
    // `tool_choice 'specified' is incompatible with thinking enabled` with a
    // 400, and DeepSeek rejects the combination outright. It outranks (1).
    let wants_thinking = compat.reasoning_effort.is_some() && compat.supports_reasoning_params;
    let forces_tool = compat.raw_tool_choice.is_some()
        || matches!(compat.tool_choice.as_deref(), Some("required") | Some("any"));
    let has_tools = tools.as_array().is_some_and(|a| !a.is_empty());
    let thinking_forbidden_by_tools = (forces_tool && compat.disable_reasoning_on_forced_tool_choice)
        || (compat.tool_choice.is_some() && compat.disable_reasoning_on_tool_choice)
        || (has_tools && compat.disable_reasoning_with_tools);
    let thinking_active = wants_thinking && !thinking_forbidden_by_tools;

    // `when_thinking` overlays a second view of the switches for requests that
    // actually think (OpenCode gateways reject `reasoning_effort` on plain
    // turns but want it while thinking). Pointer-picked, never spread.
    let wt = if thinking_active {
        compat.when_thinking.as_deref()
    } else {
        None
    };

    let sampling = wt
        .and_then(|w| w.supports_sampling_params)
        .unwrap_or(compat.supports_sampling_params);
    let penalties = wt
        .and_then(|w| w.supports_penalty_and_stop_params)
        .unwrap_or(compat.supports_penalty_and_stop_params);

    // ---- Token ceiling ----
    let tokens_field = wt
        .and_then(|w| w.max_tokens_field)
        .unwrap_or(compat.max_tokens_field);
    body.insert(tokens_field.as_str().to_string(), json!(max_tokens));

    // ---- Tools ----
    //
    // An empty roster omits the field entirely: strict gateways reject
    // `"tools": []`.
    if has_tools {
        let mut wire_tools = tools.clone();
        // `strict` 是结构化输出的开关。有的宿主不认这个字段，所以由
        // `supports_strict_mode` 门控。`None` 与 `Mixed` 都不主动加：Mixed
        // 的意思是"按每个工具自己的声明走"，而 MyPi 的工具定义里没有这个
        // 声明，所以等价于不发。
        //
        // **`strict: true` 不是"加个字段"就完事**：OpenAI 的 strict 模式对
        // schema 有两条硬要求（`openai-python` 的 `_ensure_strict_json_schema`
        // 与 omp 的 `utils/schema/normalize.ts` 都写着）：
        //
        //   1. 每个 object 节点必须有 `additionalProperties: false`；
        //   2. `properties` 里的**每一个**键都必须在 `required` 里——可选性
        //      只能靠"类型里含 null"表达。
        //
        // 缺哪条都是整个请求 400。MyPi 的工具 schema 是手写的，两条都不满
        // 足（`read` 有 `offset`/`limit` 但 `required` 只有 `path`），所以
        // 必须现场改写——这就是 [`enforce_strict_schema`]。
        if compat.supports_strict_mode
            && compat.tool_strict_mode == crate::server::ai::compat::ToolStrictMode::AllStrict
            && let Some(arr) = wire_tools.as_array_mut()
        {
            for tool in arr {
                let Some(obj) = tool.as_object_mut() else {
                    continue;
                };
                if let Some(params) = obj.get_mut("function").and_then(|f| f.get_mut("parameters")) {
                    enforce_strict_schema(params);
                }
                obj.insert("strict".to_string(), json!(true));
            }
        }
        body.insert("tools".to_string(), wire_tools);
    }

    // `tool_choice` has three downgrade paths, all documented in the compat
    // table: the host may not take the parameter at all, may not take a
    // *forced* value, or may only take the string form (llama.cpp).
    if compat.supports_tool_choice && has_tools {
        // K3 原生推理激活时不能点名某个函数（宿主 400）：保住"必须调工具"
        // 的硬约束，但让模型自己在工具里挑。
        let k3_native = compat.native_kimi_k3_reasoning && compat.reasoning_effort.is_some();
        if let Some(raw) = &compat.raw_tool_choice {
            if compat.supports_named_tool_choice && !k3_native {
                body.insert("tool_choice".to_string(), raw.clone());
            } else {
                body.insert("tool_choice".to_string(), json!("required"));
            }
        } else if let Some(choice) = compat.tool_choice.as_deref() {
            let forced = matches!(choice, "required" | "any");
            if forced && !compat.supports_forced_tool_choice {
                body.insert("tool_choice".to_string(), json!("auto"));
            } else {
                body.insert("tool_choice".to_string(), json!(choice));
            }
        }
    }

    // GLM-5.2 的 `tool_stream`：带工具时必须发，否则工具参数不以流式回来
    // （Z.ai 的 GLM-5.2 靠这个标记增量吐参数）。它跟"是不是原生
    // reasoning_effort 方言"绑在一起——同一批模型两件事都成立。
    if compat.zai_reasoning_effort_dialect
        && compat.supports_reasoning_effort
        && has_tools
    {
        body.insert("tool_stream".to_string(), json!(true));
    }

    // ---- Streaming usage ----
    if stream && compat.supports_usage_in_streaming {
        body.insert(
            "stream_options".to_string(),
            json!({ "include_usage": true }),
        );
    }

    // ---- Sampling ----
    //
    // `supports_sampling_params` is false on OpenAI's reasoning models across
    // every host (official, Azure, Copilot): they answer a 400 to `temperature`.
    if sampling {
        put_num(&mut body, "temperature", compat.temperature);
        put_num(&mut body, "top_p", compat.top_p);
        put_num(&mut body, "top_k", compat.top_k);
        put_num(&mut body, "min_p", compat.min_p);
    }
    if penalties {
        put_num(&mut body, "presence_penalty", compat.presence_penalty);
        put_num(&mut body, "frequency_penalty", compat.frequency_penalty);
        if let Some(stop) = &compat.stop {
            body.insert("stop".to_string(), json!(stop));
        }
    }
    if let Some(seed) = compat.seed {
        body.insert("seed".to_string(), json!(seed));
    }
    if let Some(user) = &compat.user {
        body.insert("user".to_string(), json!(user));
    }
    if let Some(rf) = &compat.response_format {
        body.insert("response_format".to_string(), rf.clone());
    }

    // ---- Reasoning ----
    //
    // Twelve encodings of one intent. When thinking is off, some wires still
    // need an **explicit** off signal — deleting the parameter makes the model
    // think anyway, or makes a strict host 400. That is what
    // `reasoning_disable_mode` is for.
    if thinking_active {
        let format = wt
            .and_then(|w| w.thinking_format)
            .unwrap_or(compat.thinking_format);
        let effort = compat.reasoning_effort.as_deref().map(|e| match &compat.reasoning_effort_map {
            Some(map) => map.get(e).cloned().unwrap_or_else(|| e.to_string()),
            None => e.to_string(),
        });
        let budgeting = compat.thinking_token_budget_field.is_some();
        match format {
            // `reasoning_effort: "high"`
            ThinkingFormat::Openai => {
                if let Some(e) = &effort {
                    body.insert("reasoning_effort".to_string(), json!(e));
                }
            }
            // `reasoning: { effort: "high" }`
            ThinkingFormat::Openrouter | ThinkingFormat::AntLing => {
                if let Some(e) = &effort {
                    body.insert("reasoning".to_string(), json!({ "effort": e }));
                }
            }
            // `reasoning: { enabled: true }`
            ThinkingFormat::Together => {
                body.insert("reasoning".to_string(), json!({ "enabled": true }));
            }
            // `thinking: { type: "enabled" }`
            ThinkingFormat::Deepseek => {
                body.insert("thinking".to_string(), json!({ "type": "enabled" }));
            }
            // `thinking: { type, effort }`
            ThinkingFormat::Zai | ThinkingFormat::Kimi => {
                let mut t = Map::new();
                t.insert("type".to_string(), json!("enabled"));
                if let Some(e) = &effort {
                    t.insert("effort".to_string(), json!(e));
                }
                body.insert("thinking".to_string(), Value::Object(t));
            }
            ThinkingFormat::Qwen => {
                body.insert("enable_thinking".to_string(), json!(true));
            }
            ThinkingFormat::QwenChatTemplate => {
                let mut kwargs = compat.chat_template_kwargs.clone().unwrap_or_default();
                kwargs.insert("enable_thinking".to_string(), json!(true));
                body.insert("chat_template_kwargs".to_string(), Value::Object(kwargs));
            }
            ThinkingFormat::ChatTemplate => {
                if let Some(kwargs) = &compat.chat_template_kwargs {
                    body.insert(
                        "chat_template_kwargs".to_string(),
                        Value::Object(kwargs.clone()),
                    );
                }
            }
            ThinkingFormat::Baseten => {
                if let Some(args) = &compat.chat_template_args {
                    body.insert(
                        "chat_template_args".to_string(),
                        Value::Object(args.clone()),
                    );
                }
            }
            ThinkingFormat::StringThinking => {
                if let Some(e) = &effort {
                    body.insert("thinking".to_string(), json!(e));
                }
            }
        }
        // `thinking.keep`（Z.ai / Moonshot 系）：要求宿主把历史思考留在上下文
        // 里。挂在 `thinking` 对象上，所以只有走这两种方言的宿主才有地方放。
        if let Some(keep) = &compat.thinking_keep
            && matches!(format, ThinkingFormat::Zai | ThinkingFormat::Kimi)
            && let Some(t) = body.get_mut("thinking").and_then(|v| v.as_object_mut())
        {
            t.insert("keep".to_string(), json!(keep));
        }

        // Qwen 的 chat template kwarg。不发 `preserve_thinking` 的话，新的用户
        // 消息一到，模板就把之前的 assistant 轮当成"旧的"、剥掉 think 块，
        // token 序列和缓存时的对不上，SWA 模型上就是整段重算。
        if compat.qwen_preserve_thinking || compat.qwen_template_reasoning_effort {
            let mut kwargs = body
                .get("chat_template_kwargs")
                .and_then(|v| v.as_object())
                .cloned()
                .or_else(|| compat.chat_template_kwargs.clone())
                .unwrap_or_default();
            if compat.qwen_preserve_thinking {
                kwargs.insert("preserve_thinking".to_string(), json!(true));
            }
            if compat.qwen_template_reasoning_effort
                && let Some(e) = &effort
            {
                kwargs.insert("reasoning_effort".to_string(), json!(e));
            }
            body.insert("chat_template_kwargs".to_string(), Value::Object(kwargs));
        }

        // `reasoning_effort` is a *separate* top-level field on some hosts
        // (`deepseek` + `together` accept it alongside their own dialect).
        // `omit_reasoning_effort` is the host saying it wants the dialect only.
        if !budgeting
            && compat.supports_reasoning_effort
            && !compat.omit_reasoning_effort
            && !matches!(
                format,
                ThinkingFormat::Openai
                    | ThinkingFormat::Openrouter
                    | ThinkingFormat::AntLing
            )
            && let Some(e) = &effort
        {
            body.insert("reasoning_effort".to_string(), json!(e));
        }
    } else {
        match compat.reasoning_disable_mode {
            ReasoningDisableMode::Omit => {}
            ReasoningDisableMode::LowestEffort => {
                body.insert("reasoning_effort".to_string(), json!("low"));
            }
            ReasoningDisableMode::NoneEffort => {
                body.insert("reasoning_effort".to_string(), json!("none"));
            }
            ReasoningDisableMode::OpenrouterEnabledFalse => {
                // OpenRouter 上"因为强制工具调用而关推理"时**不发**这个标记：
                // 它和 `tool_choice` 打架。语义上该关的已经关了（上面
                // `thinking_forbidden_by_tools` 挡住了 `thinking_active`），
                // 再叠一个显式 off 只是给宿主一个 400 的机会。
                if !(compat.disable_reasoning_on_forced_tool_choice && has_tools) {
                    body.insert("reasoning".to_string(), json!({ "enabled": false }));
                }
            }
            ReasoningDisableMode::ClineEnabledFalse => {
                body.insert("reasoning".to_string(), json!({ "enabled": false }));
            }
            ReasoningDisableMode::VeniceDisableThinking => {
                body.insert(
                    "venice_parameters".to_string(),
                    json!({ "disable_thinking": true }),
                );
            }
            ReasoningDisableMode::ZaiThinkingDisabled => {
                body.insert("thinking".to_string(), json!({ "type": "disabled" }));
            }
            ReasoningDisableMode::QwenEnableThinkingFalse => {
                body.insert("enable_thinking".to_string(), json!(false));
            }
            ReasoningDisableMode::QwenTemplateFalse => {
                let mut kwargs = compat.chat_template_kwargs.clone().unwrap_or_default();
                kwargs.insert("enable_thinking".to_string(), json!(false));
                body.insert("chat_template_kwargs".to_string(), Value::Object(kwargs));
            }
            ReasoningDisableMode::ChatTemplateThinkingFalse => {
                let mut kwargs = compat.chat_template_kwargs.clone().unwrap_or_default();
                kwargs.insert("thinking".to_string(), json!(false));
                body.insert("chat_template_kwargs".to_string(), Value::Object(kwargs));
            }
        }
    }

    // Reasoning-token cap: reasoning and the answer share `max_tokens` on these
    // endpoints, so a reasoning-heavy turn can eat the whole budget and emit no
    // answer at all. Only sent when the host names the field.
    if thinking_active
        && let (Some(field), Some(budget)) = (
            compat.thinking_token_budget_field,
            compat.thinking_budget,
        )
    {
        body.insert(field.as_str().to_string(), json!(budget));
    }

    // 长缓存存活期：OpenAI 的 `prompt_cache_retention: "24h"`。只对认这个
    // 字段的宿主发——别家发了多半被拒。
    if compat.supports_long_prompt_cache_retention {
        body.insert("prompt_cache_retention".to_string(), json!("24h"));
    }

    // 显式缓存断点：OpenAI 的 `prompt_cache_options` + 在最后一条**稳定的**
    // 输入消息上打标记。断点之前的整段前缀进缓存，所以打在越靠后越好；
    // 但不能打在最后一条（那一轮的内容下一轮就变了），要往前退一条。
    if compat.supports_prompt_cache_breakpoints {
        let mut opts = Map::new();
        opts.insert("mode".to_string(), json!("explicit"));
        if let Some(ttl) = &compat.prompt_cache_breakpoint_ttl {
            opts.insert("ttl".to_string(), json!(ttl));
        }
        body.insert("prompt_cache_options".to_string(), Value::Object(opts));
        if let Some(Value::Array(msgs)) = body.get_mut("messages") {
            mark_prompt_cache_breakpoint(msgs);
        }
    }

    // ---- Escape hatch ----
    //
    // Always last: whatever the user wrote wins over everything above, which is
    // what makes a vendor not in this table usable at all.
    if let Some(extra) = &compat.extra_body {
        for (k, v) in extra {
            body.insert(k.clone(), v.clone());
        }
    }

    // 最后一条规则：`reasoning_effort` 与 `thinking` 不能共存（Fireworks
    // 一类的宿主对同时出现的两者回 400）。放在 `extra_body` **之后**，这样
    // 用户手写进逃生舱的字段也被纳入判断。
    if compat.drop_thinking_when_reasoning_effort && body.contains_key("reasoning_effort") {
        body.remove("thinking");
    }

    Value::Object(body)
}


/// 把内部消息转成 wire 数组——**逐主机**决定历史里的推理内容怎么发。
///
/// 这是兼容层里最不直观的一块，因为"推理内容该不该回传"不由我们决定：
///
/// * DeepSeek V4 要求**每个** assistant 轮都带 `reasoning_content`，而且校验
///   值的真实性——塞占位符直接 400。
/// * Kimi / OpenRouter 只要求**工具调用轮**带，且接受 `.` 这样的占位符。
/// * 本地 llama.cpp 系后端**不校验**，但丢掉这个字段会让 chat template 重新
///   渲染 assistant 轮，token 序列和已有的 KV 缓存对不上，整段前缀重算。
/// * 有的宿主根本不认这个字段，多发一个就是 400。
///
/// 所以默认**一个都不发**，由 `compat` 里的开关逐项打开——与 `body_json`
/// 对可选参数的取向一致。
///
/// 顺带处理两件同属"历史消息形状"的事：工具结果消息的 `name`（有的老网关
/// 要求，而名字只能从发起它的那次调用里查回来），以及 Mistral 要求的工具
/// 调用 id 规范化。
pub(crate) fn messages_to_wire(
    messages: &[crate::server::ai::types::Message],
    compat: &crate::server::ai::compat::Resolved,
) -> serde_json::Value {
    use serde_json::{Value, json};

    // 工具消息要带 name 时，名字只能从前面那条 assistant 的 tool_calls 里查。
    let mut tool_names: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for m in messages {
        if let crate::server::ai::types::Message::Assistant { tool_calls, .. } = m {
            for c in tool_calls {
                tool_names.insert(c.id.as_str(), c.function.name.as_str());
            }
        }
    }

    let mut out: Vec<Value> = Vec::with_capacity(messages.len());
    for m in messages {
        let mut v = serde_json::to_value(m).expect("Message 序列化不会失败");
        let obj = v.as_object_mut().expect("Message 序列化成对象");

        match m {
            crate::server::ai::types::Message::Assistant {
                reasoning,
                tool_calls,
                ..
            } => {
                patch_assistant(obj, reasoning.as_deref(), !tool_calls.is_empty(), compat);
                if compat.requires_mistral_tool_ids
                    && let Some(calls) = obj.get_mut("tool_calls").and_then(|c| c.as_array_mut())
                {
                    for c in calls {
                        if let Some(slot) = c.get_mut("id") {
                            *slot = json!(mistral_tool_id(slot.as_str().unwrap_or("")));
                        }
                    }
                }
            }
            crate::server::ai::types::Message::System { .. } => {
                // 有的宿主把 system 消息叫 `developer`（OpenAI 的 o 系模型）。
                // 名字发错就是 400，而两条消息的其他部分完全一样。
                if compat.supports_developer_role
                    && obj.get("role").and_then(|r| r.as_str()) == Some("system")
                {
                    obj.insert("role".to_string(), json!("developer"));
                }
            }
            crate::server::ai::types::Message::Tool { tool_call_id, .. } => {
                if compat.requires_tool_result_name
                    && let Some(name) = tool_names.get(tool_call_id.as_str())
                {
                    obj.insert("name".to_string(), json!(name));
                }
                if compat.requires_mistral_tool_ids {
                    obj.insert(
                        "tool_call_id".to_string(),
                        json!(mistral_tool_id(tool_call_id)),
                    );
                }
            }
            _ => {}
        }
        out.push(v);
    }
    if compat.uses_openai_tool_call_id_limit {
        // OpenAI 的工具调用 id 上限是 40 字符，超过的宿主会拒。截断必须
        // **同时**作用于 assistant 里的 `tool_calls[].id` 和随后的 `tool`
        // 消息，否则配对断掉。Mistral 那条规则要求恰好 9 位，两者语义不同，
        // 由规则表保证不会同时打开。
        for m in out.iter_mut() {
            let Some(obj) = m.as_object_mut() else {
                continue;
            };
            if let Some(calls) = obj.get_mut("tool_calls").and_then(|c| c.as_array_mut()) {
                for call in calls {
                    if let Some(slot) = call.get_mut("id")
                        && let Some(s) = slot.as_str()
                        && s.len() > 40
                    {
                        *slot = json!(s[..40.min(s.len())]);
                    }
                }
            }
            if let Some(slot) = obj.get_mut("tool_call_id")
                && let Some(s) = slot.as_str()
                && s.len() > 40
            {
                *slot = json!(s[..40.min(s.len())]);
            }
        }
    }

    // 严格 chat 模板只认**一条**前导 system。false 时把有序的多段提示词拼成
    // 一条——改尾部时的 KV 缓存命中会差一点，但被拒就更糟。
    if !compat.supports_multiple_system_messages {
        let leading = out
            .iter()
            .take_while(|m| m.get("role").and_then(|r| r.as_str()) == Some("system"))
            .count();
        if leading > 1 {
            let joined = out[..leading]
                .iter()
                .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
                .collect::<Vec<_>>()
                .join("\n\n");
            let merged = serde_json::json!({ "role": "system", "content": joined });
            out.drain(..leading);
            out.insert(0, merged);
        }
    }

    // 有的宿主不接受"工具结果之后直接跟 user 消息"，要求中间夹一条 assistant。
    if compat.requires_assistant_after_tool_result {
        let mut patched: Vec<serde_json::Value> = Vec::with_capacity(out.len());
        for m in out {
            let prev_was_tool = patched
                .last()
                .and_then(|p| p.get("role"))
                .and_then(|r| r.as_str())
                == Some("tool");
            let is_user = m.get("role").and_then(|r| r.as_str()) == Some("user");
            if prev_was_tool && is_user {
                patched.push(serde_json::json!({ "role": "assistant", "content": "" }));
            }
            patched.push(m);
        }
        out = patched;
    }

    maybe_patch_images(&mut out, compat);
    maybe_add_anthropic_cache_control(&mut out, compat);
    Value::Array(out)
}

/// 图片输入的两种修补：宿主按纯文本服务时把图片换成一行占位文字，以及
/// `detail: "original"` 这个只有新实现认的取值。
///
/// 必须在缓存断点**之前**跑：断点会把 `content` 从字符串换成结构化数组，
/// 换完之后这里就再也认不出哪些部件是图片了。
fn maybe_patch_images(
    messages: &mut [serde_json::Value],
    compat: &crate::server::ai::compat::Resolved,
) {
    use serde_json::json;

    if !compat.strip_image_input && compat.supports_image_detail_original {
        return;
    }
    for m in messages.iter_mut() {
        let Some(obj) = m.as_object_mut() else {
            continue;
        };
        if obj.get("role").and_then(|r| r.as_str()) != Some("user") {
            continue;
        }
        let Some(parts) = obj.get_mut("content").and_then(|c| c.as_array_mut()) else {
            continue;
        };
        for part in parts.iter_mut() {
            if compat.strip_image_input
                && part.get("type").and_then(|t| t.as_str()) == Some("image_url")
            {
                // 占位文字而不是直接删掉：整条 content 变成空数组时，
                // 有的宿主会当成格式错误，而"这里本来有张图"对模型是有用
                // 的信息。
                *part = json!({ "type": "text", "text": "[图片已省略]" });
                continue;
            }
            let Some(detail) = part.get_mut("image_url").and_then(|i| i.get_mut("detail")) else {
                continue;
            };
            if !compat.supports_image_detail_original && detail.as_str() == Some("original") {
                *detail = json!("high");
            }
        }
    }
}

/// Anthropic 风格的显式缓存断点：在**最后一条**有正文的 user / assistant
/// 消息上打一个 `cache_control` 标记。
///
/// # 这是干什么的
///
/// 服务端的 prompt cache 让"同一段前缀第二次发过去"不用重算。OpenAI 系是
/// **隐式**的，服务端自己决定在哪断，客户端什么都不用发；Anthropic 系是
/// **显式**的，不标就一个 token 都不缓存。
///
/// 为什么断在最后一条：Anthropic 按"断点之前的最长公共前缀"算缓存，断点越
/// 靠后能命中的内容越多。为什么只要一个：Anthropic 每条请求最多 4 个断点，
/// 而 MyPi 的对话是"前缀不断增长"的形状，一个就够覆盖整段前缀。
///
/// # 为什么默认关闭
///
/// **写缓存是要额外付钱的**（Anthropic 是 +25%），读便宜 90%。断点位置放错
/// 等于白付写入费。这个开关只应该由知道自己在为什么付钱的人打开——所以它是
/// 配置里的一项，不是默认行为。
fn maybe_add_anthropic_cache_control(
    messages: &mut [serde_json::Value],
    compat: &crate::server::ai::compat::Resolved,
) {
    use crate::server::ai::compat::CacheControlFormat;
    use serde_json::json;

    if compat.cache_control_format != Some(CacheControlFormat::Anthropic) {
        return;
    }
    for m in messages.iter_mut().rev() {
        let Some(obj) = m.as_object_mut() else {
            continue;
        };
        let role = obj.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if !matches!(role, "user" | "assistant" | "developer" | "system") {
            continue;
        }
        let Some(text) = obj.get("content").and_then(|c| c.as_str()) else {
            // 已经是结构化 content（多模态）时不动它：往数组里塞标记是另一
            // 件事，且空 text 块会被 Anthropic 拒。
            continue;
        };
        // 空的 assistant 轮（纯工具调用的那一轮）**不能**转成结构化
        // content：Anthropic / OpenRouter 会拒绝空 text 块。
        if text.trim().is_empty() {
            continue;
        }
        let text = text.to_string();
        obj.insert(
            "content".to_string(),
            json!([{ "type": "text", "text": text, "cache_control": { "type": "ephemeral" } }]),
        );
        return;
    }
}

/// 一条 assistant 消息的推理内容处理。见 [`messages_to_wire`]。
fn patch_assistant(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    reasoning: Option<&str>,
    has_tool_calls: bool,
    compat: &crate::server::ai::compat::Resolved,
) {
    use serde_json::json;

    // `reasoning` is the *internal* field name. Whether it belongs on the wire
    // at all — and under which name — is what the rest of this function
    // decides, so the internal spelling must never survive: a host that wants
    // nothing would otherwise receive a field it has never heard of.
    obj.remove("reasoning");

    // 带工具调用但内容为 null 的轮：有的宿主不接受 `content: null`。
    if has_tool_calls && compat.requires_assistant_content_for_tool_calls {
        let cur = obj.get("content").and_then(|c| c.as_str()).unwrap_or("");
        obj.insert("content".to_string(), json!(cur));
    }

    let reasoning = reasoning.filter(|r| !r.is_empty());

    // 有的宿主不认独立的推理字段，只认把思考当普通文本折进 content。
    if compat.requires_thinking_as_text {
        if let Some(r) = reasoning {
            let cur = obj.get("content").and_then(|c| c.as_str()).unwrap_or("");
            let joined = if cur.is_empty() {
                r.to_string()
            } else {
                format!("{r} {cur}")
            };
            obj.insert("content".to_string(), json!(joined));
        }
        return;
    }

    let by_tools = compat.requires_reasoning_content_for_tool_calls;
    let by_all = compat.requires_reasoning_content_for_all_assistant_turns;
    let by_cache = compat.replay_reasoning_content;
    let by_dialect = compat.thinking_format == crate::server::ai::compat::ThinkingFormat::Zai;
    if !(by_tools || by_all || by_cache || by_dialect) {
        return;
    }
    // `by_tools` 只在工具调用轮需要；另外三个是"每一轮都要/都想要"。
    if !(by_all || by_cache || by_dialect) && !has_tool_calls {
        return;
    }

    let value = match reasoning {
        Some(r) => r.to_string(),
        // 没有推理内容但**必须**有这个字段时，宁可发空串也不要让宿主的校验器
        // 炸掉。`.` 这种占位符只有宿主明说接受时才敢用——DeepSeek 校验值的
        // 真实性，占位符会被拒。
        None if by_all && !compat.allows_synthetic_reasoning_content_for_tool_calls => {
            String::new()
        }
        None if has_tool_calls && compat.allows_synthetic_reasoning_content_for_tool_calls => {
            ".".to_string()
        }
        None => return,
    };
    let field = compat.reasoning_content_field.as_str();
    obj.insert(field.to_string(), json!(value));
}

/// 把一个工具的参数 schema 改写成 OpenAI strict 模式能接受的形状。
///
/// # 两条硬要求
///
/// 1. 每个 `"type": "object"` 节点必须有 `additionalProperties: false`。
///    没有的话宿主回 400，说的还是跟 schema 无关的话。
/// 2. `properties` 里的**每一个**键都必须在 `required` 里。strict 模式不
///    认"可选属性"——可选性只能用类型表达，也就是把类型变成"T 或 null"，
///    让模型用 `null` 表示"这次不传"。
///
/// # 为什么是就地改写
///
/// 这条路径只在 `toolStrictMode: allStrict` 且宿主声明认 strict 时才走。
/// 改写过的 schema 与用户写的那份不同，但它**是**发出去的那份——请求体
/// 本身就是"发出去什么"的唯一记录，所以不另存一份。
///
/// # 边界
///
/// 只看 `object` 节点、`items`、以及 `anyOf`/`oneOf`/`allOf`/`$defs` 这几
/// 个嵌套位置。MyPi 的工具 schema 是手写的简单形状，不会出现这几处以外的
/// 嵌套；真出现了也只是那一处没被改写（宿主会拒），不会静默发错。
fn enforce_strict_schema(node: &mut serde_json::Value) {
    use serde_json::{Map, Value, json};

    let Value::Object(obj) = node else {
        return;
    };

    if obj.get("type").and_then(|t| t.as_str()) == Some("object") {
        obj.insert("additionalProperties".to_string(), Value::Bool(false));

        // 先把每个属性递归改写，再决定哪些要包成可空。
        let originally_required: Vec<String> = obj
            .get("required")
            .and_then(|r| r.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();

        // 改完 properties 之后要写回 `required`，而那时 `get_mut("properties")`
        // 的借用已经结束——所以先把键名收出来，再落 insert。
        let all_keys: Vec<String> = match obj.get_mut("properties") {
            Some(Value::Object(props)) => {
                for (key, value) in props.iter_mut() {
                    enforce_strict_schema(value);
                    if originally_required.iter().any(|r| r == key) || already_nullable(value) {
                        continue;
                    }
                    // 可选 → 可空。`description` 留在外层，别埋进 union 里：
                    // 有的宿主读不到嵌套的注解，工具说明就丢了。
                    let description =
                        value.as_object_mut().and_then(|o| o.remove("description"));
                    *value = match description {
                        Some(d) => json!({
                            "anyOf": [value.clone(), { "type": "null" }],
                            "description": d
                        }),
                        None => json!({ "anyOf": [value.clone(), { "type": "null" }] }),
                    };
                }
                props.keys().cloned().collect()
            }
            _ => Vec::new(),
        };
        if !all_keys.is_empty() {
            // strict 模式要求 required 齐备且**顺序稳定**——按 properties 的
            // 声明顺序写，`Map` 是有序的，所以逐次调用得到同样的字节。
            obj.insert(
                "required".to_string(),
                Value::Array(all_keys.into_iter().map(Value::String).collect()),
            );
        }
        let _ = Map::new();
    }

    for key in ["items", "additionalProperties"] {
        if let Some(child) = obj.get_mut(key) {
            enforce_strict_schema(child);
        }
    }
    for key in ["anyOf", "oneOf", "allOf", "prefixItems"] {
        if let Some(Value::Array(branches)) = obj.get_mut(key) {
            for b in branches.iter_mut() {
                enforce_strict_schema(b);
            }
        }
    }
    if let Some(Value::Object(defs)) = obj.get_mut("$defs") {
        for (_, def) in defs.iter_mut() {
            enforce_strict_schema(def);
        }
    }
}

/// 这个节点是不是已经写成"可空"了。避免二次包一层 `anyOf`。
fn already_nullable(node: &serde_json::Value) -> bool {
    node.get("anyOf")
        .and_then(|a| a.as_array())
        .is_some_and(|branches| {
            branches
                .iter()
                .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("null"))
        })
}

/// 把会话 id 规范成 `prompt_cache_key` 能接受的长度（OpenAI 的接口约束是
/// 64 字符）。超长就换成一个稳定哈希。
///
/// **必须确定性**：同一个会话每次要得到同一个键。如果这里用了随机数或
/// 时间戳，每一轮都是新的键，缓存一个都命中不了——那比压根不发这个字段
/// 更糟，因为它看起来是配好的。
fn normalize_cache_key(id: &str) -> String {
    if id.len() <= 64 {
        return id.to_string();
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a 偏移基
    for b in id.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3); // FNV 质数
    }
    format!("pc_{h:x}")
}

/// 在最后一条**稳定的**输入消息上打显式缓存断点。
///
/// 断点之前的整段前缀进服务端缓存，所以打在越靠后越好——但最后一条 user
/// 消息是"这一轮的问题"，下一轮就变了，打在那里等于每轮都写一份新缓存。
/// 所以先找到最后一条 user/developer，再从它**往前**找第一条能缓存的。
fn mark_prompt_cache_breakpoint(messages: &mut [serde_json::Value]) {
    use serde_json::json;

    let Some(latest_input) = messages.iter().rposition(|m| {
        matches!(
            m.get("role").and_then(|r| r.as_str()),
            Some("user") | Some("developer")
        )
    }) else {
        return;
    };
    if latest_input == 0 {
        return;
    }
    for m in messages[..latest_input].iter_mut().rev() {
        let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("");
        if !matches!(role, "user" | "developer" | "system") {
            continue;
        }
        let Some(content) = m.get_mut("content") else {
            continue;
        };
        match content {
            serde_json::Value::String(text) => {
                let text = text.clone();
                let Some(obj) = m.as_object_mut() else { continue };
                obj.insert(
                    "content".to_string(),
                    json!([{
                        "type": "text",
                        "text": text,
                        "prompt_cache_breakpoint": { "mode": "explicit" }
                    }]),
                );
                return;
            }
            serde_json::Value::Array(parts) => {
                for part in parts.iter_mut().rev() {
                    let kind = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    if matches!(kind, "text" | "image_url" | "input_audio" | "file") {
                        if let Some(o) = part.as_object_mut() {
                            o.insert(
                                "prompt_cache_breakpoint".to_string(),
                                json!({ "mode": "explicit" }),
                            );
                        }
                        return;
                    }
                }
            }
            _ => {}
        }
    }
}

/// 把任意工具调用 id 规范成 Mistral 要求的"恰好 9 位字母数字"。
///
/// 必须是**确定性**的：同一个 id 每次得到同一个结果，否则 assistant 里的
/// `tool_calls[].id` 和随后的 `tool` 消息对不上，宿主判定配对失败。
fn mistral_tool_id(id: &str) -> String {
    let alnum: String = id.chars().filter(char::is_ascii_alphanumeric).collect();
    if alnum.len() == 9 {
        return alnum;
    }
    // FNV-1a：短、零依赖、跨进程稳定。
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:09}", h % 1_000_000_000)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::ai::client::Client;
    use crate::server::ai::wire::ApiUsage;
    use crate::server::ai::types::Usage;
    use serde_json::json;

    fn client() -> Client {
        Client::new("http://example.test/v1/", "k", "m")
    }

    // ---- compat: 探测、覆写、以及"字节不许变" ----

    /// 本机地址是**转发网关**，上游可能是任何东西。所以不按 llama.cpp 那套
    /// 本地特性推断（grammar schema、必回传推理内容……）。
    ///
    /// 这条锚定的是与 `oh-my-pi` 的**有意分歧**：它假设本地就是 llama.cpp，
    /// 因为它的用户直接 slap 一个在本地；MyPi 的 127.0.0.1 通常是用户自己
    /// 的网关，按 llama.cpp 推断会往云端发它不认的参数。
    #[test]
    fn a_local_gateway_gets_the_conservative_wire() {
        let r = crate::server::ai::compat::resolve("http://127.0.0.1:8788/v1", "gpt-5.6-luna", None);
        assert!(!r.supports_store, "默认不发 store");
        assert!(!r.supports_developer_role, "默认不假设认 developer 角色");
        assert!(r.supports_sampling_params, "采样参数是普遍支持的");
        assert!(r.supports_tool_choice);
        assert!(!r.strip_deepseek_special_tokens);
    }

    /// 没写 `compat:` 时请求体必须还是老样子。多一个字段就是对所有老网关的
    /// 回归——它们不接受未知参数时是以 400 拒绝，不是忽略。
    #[test]
    fn the_default_body_carries_only_the_historical_fields() {
        let c = client();
        let msgs = json!([{"role": "user", "content": "hi"}]);
        let v = body_json("m", 100, true, &msgs, &json!([]), c.compat(), None);
        let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(keys, vec!["max_tokens", "messages", "model", "stream", "stream_options"], "{v}");
        assert_eq!(v["max_tokens"], 100);
        assert_eq!(v["stream_options"]["include_usage"], true);
        assert!(v.get("tools").is_none(), "空工具表整个字段省略");
        assert!(v.get("temperature").is_none(), "没配就不发，绝不替用户编默认值");
    }

    /// 保守基线上，请求体只有这四个键——**一个新开关都不多发**。
    ///
    /// 这是 MyPi 兼容层对用户的承诺：认不出来的宿主拿到的请求，和"没有兼容
    /// 层"时逐字节相同。每个新开关都必须默认关上，这条测试是那个"默认"的
    /// 锚点——加了字段忘了改默认值，这里会失败。
    ///
    /// 用键集合而不是字符串比较：`serde_json::Map` 是 `BTreeMap`，键有序，
    /// 所以键集合相同就是字节相同。字符串字面量会随格式变化而需要重写，
    /// 键集合不会。
    #[test]
    fn the_conservative_wire_sends_nothing_extra() {
        use crate::server::ai::compat::resolve;

        // 私网地址 → 保守基线（`builtin.yaml` 没有匹配的规则）。
        let compat = resolve("http://127.0.0.1:8788/v1", "gpt-5.6-luna", None);
        let msgs = json!([{"role": "user", "content": "只回两个字：收到"}]);

        let streamed = body_json("gpt-5.6-luna", 64, true, &msgs, &json!([]), &compat, None);
        let keys: Vec<&str> = streamed.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            vec!["max_tokens", "messages", "model", "stream", "stream_options"],
            "流式请求的键集合变了：{streamed}"
        );
        assert_eq!(streamed["stream_options"], json!({"include_usage": true}));

        let plain = body_json("gpt-5.6-luna", 64, false, &msgs, &json!([]), &compat, None);
        let keys: Vec<&str> = plain.as_object().unwrap().keys().map(String::as_str).collect();
        assert_eq!(
            keys,
            vec!["max_tokens", "messages", "model", "stream"],
            "非流式请求的键集合变了：{plain}"
        );
    }

    /// 会话 id 只在宿主声明要缓存键时才出现在请求里。
    #[test]
    fn the_prompt_cache_key_needs_both_the_switch_and_a_session() {
        use crate::server::ai::compat::resolve;

        let msgs = json!([{"role": "user", "content": "hi"}]);
        let mut compat = resolve("https://api.openai.com/v1", "gpt-5.6", None);

        // 开关关着：会话 id 给了也不发。
        compat.supports_prompt_cache_key = false;
        let v = body_json("gpt-5.6", 64, false, &msgs, &json!([]), &compat, Some("s-1"));
        assert!(v.get("prompt_cache_key").is_none(), "没开就不该发：{v}");

        // 开关开了但没有会话 id：也不发（空的键会把所有请求归到一起）。
        compat.supports_prompt_cache_key = true;
        let v = body_json("gpt-5.6", 64, false, &msgs, &json!([]), &compat, None);
        assert!(v.get("prompt_cache_key").is_none(), "没有会话 id 就不该发：{v}");

        // 两个都齐：发，而且是确定性的。
        let v = body_json("gpt-5.6", 64, false, &msgs, &json!([]), &compat, Some("s-1"));
        assert_eq!(v["prompt_cache_key"], "s-1");
        let again = body_json("gpt-5.6", 64, false, &msgs, &json!([]), &compat, Some("s-1"));
        assert_eq!(v, again, "同一个会话必须得到同一个键，否则缓存一次都命中不了");
    }

    /// 超过 64 字符的会话 id 折成稳定哈希，不能原样截断。
    #[test]
    fn an_overlong_cache_key_is_hashed_not_truncated() {
        let long = "x".repeat(200);
        let a = normalize_cache_key(&long);
        let b = normalize_cache_key(&long);
        assert!(a.len() <= 64, "键要落在上限内：{}", a.len());
        assert_eq!(a, b, "同一个输入必须得到同一个键");
        // 截断会让两个只有尾巴不同的会话撞成一个键，那等于所有会话共用一份
        // 缓存——比不发这个字段更糟。
        let other = format!("{}y", "x".repeat(199));
        assert_ne!(a, normalize_cache_key(&other), "不同的会话不能撞键");
        assert_eq!(normalize_cache_key("short"), "short", "够短的会话 id 原样用");
    }

    /// `reasoning_effort` 与 `thinking` 不能共存时，后者要让路。
    ///
    /// Fireworks 一类的宿主对同时出现的两者回 400。判断必须在 `extra_body`
    /// 合并**之后**——用户完全可能把 `reasoning_effort` 写进逃生舱。
    #[test]
    fn reasoning_effort_evicts_thinking_when_the_host_demands_it() {
        use crate::server::ai::compat::{Compat, resolve};

        let msgs = json!([{"role": "user", "content": "hi"}]);
        // Z.ai 的方言：`thinking: { type: "enabled" }` 和顶层 `reasoning_effort`
        // 会同时发出去，正是宿主会拒的那一对。
        let user: Compat = serde_yaml::from_str(
            "reasoningEffort: high\nthinkingFormat: zai\nsupportsReasoningEffort: true\n\
             dropThinkingWhenReasoningEffort: true\n",
        )
        .unwrap();
        let compat = resolve("https://api.z.ai/v1", "glm-4.6", Some(&user));

        let v = body_json("glm-4.6", 64, false, &msgs, &json!([]), &compat, None);
        assert_eq!(v["reasoning_effort"], "high", "effort 还在：{v}");
        assert!(v.get("thinking").is_none(), "thinking 必须让路：{v}");

        // 关掉开关时两个都发（那是宿主的默认期望）。
        let user2: Compat =
            serde_yaml::from_str(
                "reasoningEffort: high\nthinkingFormat: zai\nsupportsReasoningEffort: true\n",
            )
            .unwrap();
        let compat2 = resolve("https://api.z.ai/v1", "glm-4.6", Some(&user2));
        let v2 = body_json("glm-4.6", 64, false, &msgs, &json!([]), &compat2, None);
        assert!(v2.get("thinking").is_some(), "没开开关就照常发：{v2}");
    }

    /// 逃生舱里手写的 `reasoning_effort` 同样触发上面那条规则。
    #[test]
    fn the_escape_hatch_is_covered_by_the_same_rule() {
        use crate::server::ai::compat::{Compat, resolve};

        let msgs = json!([{"role": "user", "content": "hi"}]);
        let user: Compat = serde_yaml::from_str(
            "reasoningEffort: high\nthinkingFormat: zai\nsupportsReasoningEffort: true\n\
             dropThinkingWhenReasoningEffort: true\nextraBody:\n  reasoning_effort: medium\n",
        )
        .unwrap();
        let compat = resolve("https://api.z.ai/v1", "glm-4.6", Some(&user));
        let v = body_json("glm-4.6", 64, false, &msgs, &json!([]), &compat, None);
        assert_eq!(v["reasoning_effort"], "medium", "逃生舱的值优先");
        assert!(v.get("thinking").is_none(), "逃生舱写的 effort 也算数：{v}");
    }

    /// `strict: true` 必须**同时**把 schema 改写成 strict 认的形状。
    ///
    /// 这是补上的一个真实缺陷：MyPi 以前只往工具上插 `"strict": true`，
    /// 而 OpenAI 的 strict 模式另有两条硬要求（每个 object 节点要有
    /// `additionalProperties: false`；`properties` 的每个键都要在
    /// `required` 里）。缺任一条整个请求 400——而用户开这个开关的**意思**
    /// 就是"让模型严格按 schema 回"，正是最不该失败的一档。
    ///
    /// 用真实的工具表，不手搓 schema：手搓的那份不会跟着工具一起变。
    #[test]
    fn strict_mode_rewrites_the_schema_not_just_the_flag() {
        use crate::server::ai::compat::{ToolStrictMode, resolve};

        let tools = serde_json::to_value(
            crate::server::agent::tools::BuiltinTools::definitions(),
        )
        .unwrap();

        let mut compat = resolve("https://api.openai.com/v1", "gpt-5.6", None);
        compat.supports_strict_mode = true;
        compat.tool_strict_mode = ToolStrictMode::AllStrict;

        let body = body_json("gpt-5.6", 100, true, &json!([]), &tools, &compat, None);
        let arr = body["tools"].as_array().expect("有工具");
        assert!(!arr.is_empty(), "工具表不该是空的");

        for tool in arr {
            let name = tool["function"]["name"].as_str().unwrap_or("?");
            assert_eq!(tool["strict"], true, "{name} 应有 strict");
            let params = &tool["function"]["parameters"];
            assert_eq!(
                params["additionalProperties"], false,
                "{name}: strict 要求每个 object 节点有 additionalProperties: false"
            );
            let props = params["properties"].as_object().expect("有 properties");
            let required: Vec<&str> = params["required"]
                .as_array()
                .expect("有 required")
                .iter()
                .filter_map(|v| v.as_str())
                .collect();
            for key in props.keys() {
                assert!(
                    required.contains(&key.as_str()),
                    "{name}: strict 要求 `{key}` 也在 required 里（可选性要靠类型表达）"
                );
            }
        }

        // 抽一个已知有可选字段的工具（`read` 的 offset/limit），确认它被包成
        // 可空——只补 required 而不改类型的话，模型就没有"不传"的表达方式了。
        let read = arr
            .iter()
            .find(|t| t["function"]["name"] == "read")
            .expect("read 在工具表里");
        let offset = &read["function"]["parameters"]["properties"]["offset"];
        assert!(
            offset["anyOf"]
                .as_array()
                .is_some_and(|b| b.iter().any(|v| v["type"] == "null")),
            "可选的 offset 要包成 anyOf 含 null：{offset}"
        );
        assert_eq!(
            read["function"]["parameters"]["properties"]["path"]["type"], "string",
            "必填项不该被包"
        );
    }

    /// 没开 strict 时 schema **原样**发出去。
    ///
    /// 改写是有代价的（多一层 `anyOf`、模型多一个 `null` 的分支），不能
    /// 顺手对所有宿主都做。
    #[test]
    fn the_schema_is_untouched_unless_strict_is_on() {
        use crate::server::ai::compat::resolve;

        let tools = serde_json::to_value(
            crate::server::agent::tools::BuiltinTools::definitions(),
        )
        .unwrap();
        // 私网地址拿的是保守基线（`builtin.yaml` 里没有匹配的规则）：不发
        // strict，所以 schema 也不该被动。
        let compat = resolve("http://127.0.0.1:8788/v1", "gpt-5.6-luna", None);
        assert!(!compat.supports_strict_mode, "保守基线不发 strict");

        let body = body_json("gpt-5.6-luna", 100, true, &json!([]), &tools, &compat, None);
        let read = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["function"]["name"] == "read")
            .unwrap();
        assert!(
            read["function"]["parameters"]["additionalProperties"].is_null(),
            "没开 strict 就不该动 schema"
        );
        assert!(read["strict"].is_null(), "没开 strict 就不该发这个字段");
        // `required` 原样：可选字段**没有**被补进去（补进去就说明改写了）。
        let required = read["function"]["parameters"]["required"]
            .as_array()
            .expect("有 required");
        let names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(names.contains(&"path"), "必填项在 required 里");
        assert!(
            !names.contains(&"offset") && !names.contains(&"limit"),
            "没开 strict 就不该把可选字段塞进 required：{names:?}"
        );
    }

    /// 用户写的开关逐项覆盖探测结果，没写的项保持探测值。
    #[test]
    fn user_switches_override_detection() {
        use crate::server::ai::compat::{MaxTokensField, resolve};
        let user: crate::server::ai::compat::Compat =
            serde_yaml::from_str("temperature: 0.3\nmaxTokensField: max_completion_tokens\n")
                .unwrap();
        let r = resolve("https://api.deepseek.com/v1", "deepseek-chat", Some(&user));
        assert_eq!(r.max_tokens_field, MaxTokensField::MaxCompletionTokens, "覆盖生效");
        assert_eq!(r.temperature, Some(0.3));
        assert!(r.strip_deepseek_special_tokens, "没被覆盖的探测结论还在");
        assert!(r.disable_reasoning_on_tool_choice, "同上");
    }

    /// OpenAI 的推理模型在**所有**托管方都拒绝 `temperature`
    /// （`400 Unsupported parameter`）。同一个用户配置换个宿主就该发出去。
    #[test]
    fn sampling_parameters_follow_the_host_not_the_user() {
        use crate::server::ai::compat::resolve;
        let user: crate::server::ai::compat::Compat =
            serde_yaml::from_str("temperature: 0.9\n").unwrap();

        let openai = resolve("https://api.openai.com/v1", "gpt-5.6", Some(&user));
        assert!(!openai.supports_sampling_params);
        let v = body_json("gpt-5.6", 100, false, &json!([]), &json!([]), &openai, None);
        assert!(v.get("temperature").is_none(), "该拒的宿主不许发：{v}");
        assert_eq!(v["max_completion_tokens"], 100, "官方新模型只认这个字段名");
        assert!(v.get("max_tokens").is_none());

        let deepseek = resolve("https://api.deepseek.com/v1", "deepseek-chat", Some(&user));
        let v2 = body_json("deepseek-chat", 100, false, &json!([]), &json!([]), &deepseek, None);
        assert_eq!(v2["temperature"], 0.9);
        assert_eq!(v2["max_tokens"], 100);
    }

    /// `extra_body` 是逃生舱：表里没有的服务商靠它发非标准参数，所以它必须
    /// **最后**合并，用户写的键优先于前面所有装配结果。
    #[test]
    fn extra_body_wins_over_everything() {
        use crate::server::ai::compat::resolve;
        let user: crate::server::ai::compat::Compat = serde_yaml::from_str(
            "temperature: 0.3\nextraBody:\n  temperature: 0.99\n  my_gateway_hint: route-a\n",
        )
        .unwrap();
        let r = resolve("http://127.0.0.1:8788/v1", "gpt-5.6-luna", Some(&user));
        let v = body_json("gpt-5.6-luna", 100, false, &json!([]), &json!([]), &r, None);
        assert_eq!(v["temperature"], 0.99, "逃生舱覆盖装配结果");
        assert_eq!(v["my_gateway_hint"], "route-a");
    }

    /// 推理参数的形状随 `thinking_format` 变；用户要的档位落在对的位置上。
    #[test]
    fn thinking_parameters_take_the_host_dialect() {
        use crate::server::ai::compat::{Compat, resolve};
        let want: Compat = serde_yaml::from_str("reasoningEffort: high\n").unwrap();
        let msgs = json!([{"role": "user", "content": "hi"}]);

        // OpenAI 官方：顶层 reasoning_effort
        let r = resolve("https://api.openai.com/v1", "gpt-5.6", Some(&want));
        assert_eq!(body_json("gpt-5.6", 100, false, &msgs, &json!([]), &r, None)["reasoning_effort"], "high");

        // OpenRouter：reasoning: { effort }
        let r = resolve("https://openrouter.ai/api/v1", "x/y", Some(&want));
        assert_eq!(body_json("x/y", 100, false, &msgs, &json!([]), &r, None)["reasoning"]["effort"], "high");

        // Qwen：顶层 enable_thinking 布尔
        let r = resolve("https://dashscope.aliyuncs.com/compatible-mode/v1", "qwen3-max", Some(&want));
        assert_eq!(body_json("qwen3-max", 100, false, &msgs, &json!([]), &r, None)["enable_thinking"], true);

        // 没要推理时一个字都不发
        let r = resolve("https://api.openai.com/v1", "gpt-5.6", None);
        let v = body_json("gpt-5.6", 100, false, &msgs, &json!([]), &r, None);
        assert!(v.get("reasoning_effort").is_none(), "{v}");
    }

    /// 缓存读取数有三个可能的落点，取**第一个正数**。
    ///
    /// 这不是洁癖：有的宿主把用不着的那个字段填成显式 `0`，先到先得就会
    /// 把真值屏蔽掉，而缓存列是**钱**——静默少报比报错更糟。
    #[test]
    fn cache_reads_are_read_from_whichever_field_the_host_uses() {
        let usage_of = |j: serde_json::Value| Usage::from(serde_json::from_value::<ApiUsage>(j).unwrap());
        let base = json!({"prompt_tokens": 100, "completion_tokens": 1, "total_tokens": 101});

        let mut top = base.clone();
        top["cached_tokens"] = json!(64);
        assert_eq!(usage_of(top).cached_tokens, Some(64), "只发顶层的宿主");

        let mut nested = base.clone();
        nested["prompt_tokens_details"] = json!({"cached_tokens": 80});
        assert_eq!(usage_of(nested).cached_tokens, Some(80), "只发嵌套的宿主");

        let mut ds = base.clone();
        ds["prompt_cache_hit_tokens"] = json!(96);
        assert_eq!(usage_of(ds).cached_tokens, Some(96), "DeepSeek 拼写");

        let mut shadowed = base.clone();
        shadowed["cached_tokens"] = json!(0);
        shadowed["prompt_tokens_details"] = json!({"cached_tokens": 72});
        assert_eq!(usage_of(shadowed).cached_tokens, Some(72), "显式 0 不许屏蔽真值");

        assert_eq!(usage_of(base).cached_tokens, None, "真没有就是没有");
    }

    #[test]
    fn streaming_body_requests_a_usage_block() {
        // The only difference between non-streaming and streaming is
        // stream + stream_options. This behavior once had no test — and
        // with `complete` and `stream` each building a body, this is
        // where mistakes happen (forgetting include_usage on the
        // streaming side silently zeroes the cost bar).
        let c = client();
        let msgs = json!([]);
        let tools = json!([]);
        let stream = body_json(c.model(), 100, true, &msgs, &tools, c.compat(), None);
        assert_eq!(stream["stream"], json!(true));
        assert_eq!(
            stream["stream_options"]["include_usage"],
            json!(true),
            "streaming must request the usage block or pricing gets no data"
        );

        let plain = body_json(c.model(), 100, false, &msgs, &tools, c.compat(), None);
        assert_eq!(plain["stream"], json!(false));
        assert!(
            plain.get("stream_options").is_none(),
            "non-streaming must not send stream_options"
        );
        // Without tools the whole field must be omitted, not an empty array
        assert!(plain.get("tools").is_none());
    }

    #[test]
    fn tools_field_is_present_only_when_defined() {
        let c = client();
        let msgs = json!([]);
        let tools = json!([{
            "type": "function",
            "function": {"name": "read_file", "description": "read a file",
                         "parameters": {"type": "object"}}
        }]);
        let v = body_json(c.model(), 100, false, &msgs, &tools, c.compat(), None);
        assert_eq!(v["tools"][0]["type"], "function");
        assert_eq!(v["tools"][0]["function"]["name"], "read_file");
    }


    // ---- 历史消息的 wire 形状：逐主机决定，推理内容默认不上线 ----

    use crate::server::ai::compat::{ReasoningContentField, Resolved, ThinkingFormat};
    use crate::server::ai::types::{Message, ToolCall};

    fn assistant(content: Option<&str>, calls: Vec<ToolCall>, reasoning: Option<&str>) -> Message {
        Message::Assistant {
            content: content.map(str::to_string),
            tool_calls: calls,
            reasoning: reasoning.map(str::to_string),
        }
    }

    fn tool(call_id: &str, name: &str) -> Message {
        let _ = name;
        Message::Tool {
            tool_call_id: call_id.to_string(),
            content: "ok".to_string(),
        }
    }

    /// 默认一个字段都不多发。老网关收到没听过的参数是以 400 拒绝，不是忽略。
    #[test]
    fn history_carries_no_reasoning_unless_the_host_asks() {
        let r = Resolved::conservative();
        let wire = messages_to_wire(
            &[
                assistant(Some("答"), vec![], Some("内心")),
                tool("c1", "bash"),
            ],
            &r,
        );
        let text = serde_json::to_string(&wire).unwrap();
        assert!(!text.contains("内心"), "{text}");
        assert!(
            !wire[0].as_object().unwrap().contains_key("reasoning"),
            "内部字段名不得泄漏到 wire：{text}"
        );
    }

    /// DeepSeek V4：**每个** assistant 轮都要 `reasoning_content`，而且校验值
    /// 的真实性——没有推理内容时也不许塞占位符。
    #[test]
    fn deepseek_style_hosts_get_reasoning_on_every_assistant_turn() {
        let mut r = Resolved::conservative();
        r.thinking_format = ThinkingFormat::Deepseek;
        r.reasoning_content_field = ReasoningContentField::ReasoningContent;
        r.requires_reasoning_content_for_tool_calls = true;
        r.requires_reasoning_content_for_all_assistant_turns = true;
        r.allows_synthetic_reasoning_content_for_tool_calls = false;

        let wire = messages_to_wire(
            &[
                assistant(Some("先查"), vec![], Some("我该看看文件")),
                assistant(Some("答完了"), vec![], None),
            ],
            &r,
        );
        assert_eq!(wire[0]["reasoning_content"], "我该看看文件");
        assert_eq!(
            wire[1]["reasoning_content"], "",
            "缺字段会被校验器拒；空串是诚实的表示"
        );
        assert!(wire[0].get("reasoning").is_none(), "内部名必须被换掉");
    }

    /// Kimi / OpenRouter：只有工具调用轮需要，且接受 `.` 占位符。
    #[test]
    fn hosts_that_take_a_placeholder_get_one_only_on_tool_turns() {
        let mut r = Resolved::conservative();
        r.requires_reasoning_content_for_tool_calls = true;
        r.allows_synthetic_reasoning_content_for_tool_calls = true;

        let wire = messages_to_wire(
            &[
                assistant(Some("查"), vec![ToolCall::new("c1", "bash", "{}")], None),
                assistant(Some("答"), vec![], None),
            ],
            &r,
        );
        assert_eq!(wire[0]["reasoning_content"], ".");
        assert!(
            wire[1].get("reasoning_content").is_none(),
            "纯文本轮不需要，就不要多发一个字段"
        );
    }

    /// 有的宿主不认独立的推理字段，只认把思考折进可见文本。
    #[test]
    fn hosts_that_want_thinking_as_text_get_it_folded_in() {
        let mut r = Resolved::conservative();
        r.requires_thinking_as_text = true;

        let wire = messages_to_wire(&[assistant(Some("答案"), vec![], Some("想过"))], &r);
        assert_eq!(wire[0]["content"], "想过 答案");
        assert!(wire[0].get("reasoning_content").is_none());
    }

    /// 老网关要 `name`，而名字只能从发起那次调用的 assistant 里查回来。
    #[test]
    fn tool_results_gain_their_name_when_the_host_wants_it() {
        let mut r = Resolved::conservative();
        r.requires_tool_result_name = true;

        let wire = messages_to_wire(
            &[
                assistant(None, vec![ToolCall::new("c1", "read_file", "{}")], None),
                tool("c1", "read_file"),
            ],
            &r,
        );
        assert_eq!(wire[1]["name"], "read_file");
        assert_eq!(wire[1]["tool_call_id"], "c1");
    }

    /// Mistral：工具调用 id 必须恰好 9 位字母数字，而且 assistant 里的
    /// `tool_calls[].id` 与随后的 `tool` 消息必须**同时**改，否则配对失败。
    #[test]
    fn mistral_ids_are_normalized_on_both_sides_of_the_pair() {
        let mut r = Resolved::conservative();
        r.requires_mistral_tool_ids = true;

        let wire = messages_to_wire(
            &[
                assistant(None, vec![ToolCall::new("call_abc123def456", "f", "{}")], None),
                tool("call_abc123def456", "f"),
            ],
            &r,
        );
        let id = wire[0]["tool_calls"][0]["id"].as_str().unwrap();
        assert_eq!(id.len(), 9, "{id}");
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric()), "{id}");
        assert_eq!(wire[1]["tool_call_id"], id, "两边必须是同一个 id");

        // 确定性：同一个输入再跑一次得到同一个结果，否则回放会漂移。
        let again = messages_to_wire(
            &[
                assistant(None, vec![ToolCall::new("call_abc123def456", "f", "{}")], None),
                tool("call_abc123def456", "f"),
            ],
            &r,
        );
        assert_eq!(again[1]["tool_call_id"], id);
    }

    /// 带工具调用但内容为空的轮：有的宿主不接受 `content: null`。
    #[test]
    fn some_hosts_reject_a_null_content_on_tool_turns() {
        let mut r = Resolved::conservative();
        r.requires_assistant_content_for_tool_calls = true;

        let wire = messages_to_wire(
            &[assistant(None, vec![ToolCall::new("c1", "f", "{}")], None)],
            &r,
        );
        assert_eq!(wire[0]["content"], "");
    }


    // ---- 显式缓存断点 ----

    /// Anthropic 系不标就一个 token 都不缓存。断点打在**最后一条**有正文的
    /// 消息上：Anthropic 按"断点之前的最长公共前缀"算，断点越靠后覆盖越多。
    #[test]
    fn anthropic_style_hosts_get_one_cache_breakpoint_late_in_the_tail() {
        use crate::server::ai::compat::CacheControlFormat;
        let mut r = Resolved::conservative();
        assert!(
            messages_to_wire(&[Message::User { content: "问".into() }], &r)[0]
                .get("content")
                .is_some_and(|c| c.is_string()),
            "默认不打标记，content 保持字符串"
        );

        r.cache_control_format = Some(CacheControlFormat::Anthropic);
        let wire = messages_to_wire(
            &[
                Message::User { content: "第一问".into() },
                assistant(Some("第一答"), vec![], None),
                Message::User { content: "第二问".into() },
            ],
            &r,
        );
        assert!(wire[0]["content"].is_string(), "只打一个，靠前的不动");
        assert_eq!(wire[2]["content"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(wire[2]["content"][0]["text"], "第二问");
    }

    /// 空的 assistant 轮（纯工具调用）不能转成结构化 content——Anthropic 和
    /// OpenRouter 都会拒绝空的 text 块。断点要往回找到有正文的那条。
    #[test]
    fn a_cache_breakpoint_skips_empty_reply_turns() {
        use crate::server::ai::compat::CacheControlFormat;
        let mut r = Resolved::conservative();
        r.cache_control_format = Some(CacheControlFormat::Anthropic);

        let wire = messages_to_wire(
            &[
                Message::User { content: "查一下".into() },
                assistant(None, vec![ToolCall::new("c1", "bash", "{}")], None),
                tool("c1", "bash"),
            ],
            &r,
        );
        assert!(wire[1]["content"].is_null(), "空轮保持 null，不塞空 text 块");
        assert_eq!(wire[0]["content"][0]["cache_control"]["type"], "ephemeral");
    }

    // ---- strict ----

    /// `strict` 只在宿主认这个字段、且规则说"全部严格"时才加。
    #[test]
    fn strict_is_added_only_where_the_host_understands_it() {
        use crate::server::ai::compat::ToolStrictMode;
        let msgs = serde_json::json!([{"role": "user", "content": "hi"}]);
        let tools = serde_json::json!([
            {"type": "function", "function": {"name": "f", "description": "", "parameters": {}}}
        ]);

        let mut r = Resolved::conservative();
        r.tool_strict_mode = ToolStrictMode::AllStrict;
        // 宿主认不认这个字段由 supports_strict_mode 说，不认就不发。
        let v = body_json("m", 100, false, &msgs, &tools, &r, None);
        assert!(v["tools"][0].get("strict").is_none(), "宿主不认就不发");

        r.supports_strict_mode = true;
        let v = body_json("m", 100, false, &msgs, &tools, &r, None);
        assert_eq!(v["tools"][0]["strict"], true);
        // 是**复制**后改的：调用方传进来的 tools 不能被就地改掉。
        assert!(tools[0].get("strict").is_none(), "不得就地修改调用方的 tools");
    }


    // ---- 多模态 ----

    use crate::server::ai::types::{ContentPart, ImageUrl, UserContent};

    /// 不带图的请求必须与"图片功能存在之前"**逐字节**一致：形状一变，
    /// 服务端认得的整段前缀就都失效了。
    #[test]
    fn a_text_only_message_still_serializes_as_a_bare_string() {
        let r = Resolved::conservative();
        let wire = messages_to_wire(&[Message::User { content: "你好".into() }], &r);
        assert!(wire[0]["content"].is_string(), "{wire}");
        assert_eq!(wire[0]["content"], "你好");
    }

    #[test]
    fn an_image_message_serializes_as_parts() {
        let r = Resolved::conservative();
        let msg = Message::User {
            content: UserContent::Parts(vec![
                ContentPart::Text {
                    text: "这是什么".into(),
                },
                ContentPart::ImageUrl {
                    image_url: ImageUrl {
                        url: "data:image/png;base64,AAA".into(),
                        detail: None,
                    },
                },
            ]),
        };
        let wire = messages_to_wire(&[msg], &r);
        assert_eq!(wire[0]["content"][0]["type"], "text");
        assert_eq!(wire[0]["content"][1]["type"], "image_url");
        assert_eq!(wire[0]["content"][1]["image_url"]["url"], "data:image/png;base64,AAA");
        assert!(
            wire[0]["content"][1]["image_url"].get("detail").is_none(),
            "没写就不发，不替用户编一个"
        );
    }

    /// 按纯文本服务图片的宿主：图片换成占位文字，而不是留一个它不认的部件。
    #[test]
    fn hosts_that_serve_text_only_get_a_placeholder_instead_of_an_image() {
        let mut r = Resolved::conservative();
        r.strip_image_input = true;
        let msg = Message::User {
            content: UserContent::Parts(vec![
                ContentPart::Text {
                    text: "看图".into(),
                },
                ContentPart::ImageUrl {
                    image_url: ImageUrl {
                        url: "data:image/png;base64,AAA".into(),
                        detail: None,
                    },
                },
            ]),
        };
        let wire = messages_to_wire(&[msg], &r);
        assert_eq!(wire[0]["content"][1]["type"], "text");
        assert!(wire[0]["content"][1]["text"].as_str().unwrap().contains("省略"));
        let text = serde_json::to_string(&wire).unwrap();
        assert!(
            !text.contains("base64"),
            "图片数据必须消失——那才是这个开关的全部意义"
        );
    }

    /// `detail: "original"` 只有新实现认，老宿主会拒绝。只有**明说**认的
    /// 宿主才留得住它。
    #[test]
    fn a_host_that_rejects_original_detail_gets_high_instead() {
        let mut r = Resolved::conservative();
        let msg = || Message::User {
            content: UserContent::Parts(vec![ContentPart::ImageUrl {
                image_url: ImageUrl {
                    url: "https://x/y.png".into(),
                    detail: Some("original".into()),
                },
            }]),
        };

        let fixed = messages_to_wire(&[msg()], &r);
        assert_eq!(fixed[0]["content"][0]["image_url"]["detail"], "high");

        r.supports_image_detail_original = true;
        let kept = messages_to_wire(&[msg()], &r);
        assert_eq!(kept[0]["content"][0]["image_url"]["detail"], "original");

        // 其他取值一律不动。
        for d in ["auto", "low", "high"] {
            let m = Message::User {
                content: UserContent::Parts(vec![ContentPart::ImageUrl {
                    image_url: ImageUrl {
                        url: "https://x/y.png".into(),
                        detail: Some(d.into()),
                    },
                }]),
            };
            let wire = messages_to_wire(&[m], &r);
            assert_eq!(wire[0]["content"][0]["image_url"]["detail"], d);
        }
    }


    // ---- 这一轮才落地的几个开关 ----

    #[test]
    fn thinking_keep_rides_on_the_thinking_object() {
        use crate::server::ai::compat::{ThinkingKeep, ThinkingKeepAll};
        let r = Resolved {
            thinking_format: ThinkingFormat::Zai,
            thinking_keep: Some(ThinkingKeep::All(ThinkingKeepAll::All)),
            reasoning_effort: Some("high".into()),
            ..Resolved::conservative()
        };
        let v = body_json("m", 100, false, &json!([{"role":"user","content":"hi"}]), &json!([]), &r, None);
        assert_eq!(v["thinking"]["type"], "enabled");
        assert_eq!(v["thinking"]["keep"], "all");
    }

    /// Qwen 的 chat template kwarg：不发 `preserve_thinking`，模板会剥掉历史
    /// think 块，token 序列和缓存时的对不上。
    #[test]
    fn qwen_template_kwargs_go_out_for_the_dialects_that_read_them() {
        let r = Resolved {
            thinking_format: ThinkingFormat::QwenChatTemplate,
            reasoning_effort: Some("high".into()),
            qwen_preserve_thinking: true,
            qwen_template_reasoning_effort: true,
            ..Resolved::conservative()
        };
        let v = body_json("m", 100, false, &json!([{"role":"user","content":"hi"}]), &json!([]), &r, None);
        assert_eq!(v["chat_template_kwargs"]["preserve_thinking"], true);
        assert_eq!(v["chat_template_kwargs"]["reasoning_effort"], "high");
    }

    /// 长缓存存活期只对认这个字段的宿主发：别家发了多半被拒。
    #[test]
    fn long_cache_retention_is_only_asked_for_where_it_is_understood() {
        let msgs = json!([{"role":"user","content":"hi"}]);
        let off = Resolved::conservative();
        let v = body_json("m", 100, false, &msgs, &json!([]), &off, None);
        assert!(v.get("prompt_cache_retention").is_none(), "{v}");

        let on = Resolved { supports_long_prompt_cache_retention: true, ..Resolved::conservative() };
        let v = body_json("m", 100, false, &msgs, &json!([]), &on, None);
        assert_eq!(v["prompt_cache_retention"], "24h");
    }

    /// 严格 chat 模板只认一条前导 system：多条要拼成一条，顺序保住。
    #[test]
    fn multiple_system_messages_are_merged_for_strict_templates() {
        let r = Resolved::conservative();
        let wire = messages_to_wire(
            &[
                Message::System { content: "甲".into() },
                Message::System { content: "乙".into() },
                Message::User { content: "问".into() },
            ],
            &r,
        );
        assert_eq!(wire.as_array().unwrap().len(), 2, "两条 system 拼成一条");
        assert_eq!(wire[0]["role"], "system");
        assert_eq!(wire[0]["content"], "甲\n\n乙");

        let keep = Resolved { supports_multiple_system_messages: true, ..Resolved::conservative() };
        let wire = messages_to_wire(
            &[
                Message::System { content: "甲".into() },
                Message::System { content: "乙".into() },
                Message::User { content: "问".into() },
            ],
            &keep,
        );
        assert_eq!(wire.as_array().unwrap().len(), 3, "认多条就不动它");
    }

    /// 有的宿主不接受"工具结果后直接跟 user"，要求中间夹一条 assistant。
    #[test]
    fn a_blank_assistant_is_inserted_after_tool_results_when_demanded() {
        let r = Resolved {
            requires_assistant_after_tool_result: true,
            ..Resolved::conservative()
        };
        let wire = messages_to_wire(
            &[
                assistant(None, vec![ToolCall::new("c1", "f", "{}")], None),
                tool("c1", "f"),
                Message::User { content: "继续".into() },
            ],
            &r,
        );
        assert_eq!(wire[2]["role"], "assistant");
        assert_eq!(wire[2]["content"], "");
        assert_eq!(wire[3]["role"], "user");
    }

    /// 工具调用 id 超过 40 字符时，assistant 与 tool 两边必须一起截。
    #[test]
    fn overlong_tool_ids_are_truncated_on_both_sides() {
        let long = "call_".to_string() + &"a".repeat(60);
        let r = Resolved {
            uses_openai_tool_call_id_limit: true,
            ..Resolved::conservative()
        };
        let wire = messages_to_wire(
            &[
                assistant(None, vec![ToolCall::new(&long, "f", "{}")], None),
                tool(&long, "f"),
            ],
            &r,
        );
        let id = wire[0]["tool_calls"][0]["id"].as_str().unwrap();
        assert_eq!(id.len(), 40);
        assert_eq!(wire[1]["tool_call_id"], id, "两边一起截，否则配对断掉");
    }
}
