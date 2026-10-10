//! Request body assembly: `(model, max_tokens, stream, messages, tools, compat)` → one JSON object.
//! No IO, no global state, no local config.

/// The single construction point for a request body: the live path and the rebuild path both come here,
/// so a rebuilt request cannot drift. Field names come from `compat`.
pub(crate) fn body_json(
    model: &str,
    max_tokens: u32,
    stream: bool,
    messages: &serde_json::Value,
    tools: &serde_json::Value,
    compat: &crate::server::agent::request::assembly::compat::Resolved,
    session_id: Option<&str>,
) -> serde_json::Value {
    use crate::server::agent::request::assembly::compat::{ReasoningDisableMode, ThinkingFormat};
    use serde_json::{json, Map, Value};

    fn put_num(body: &mut Map<String, Value>, key: &str, v: Option<f64>) {
        if let Some(x) = v {
            body.insert(key.to_string(), json!(x));
        }
    }

    let mut body: Map<String, Value> = Map::new();
    body.insert("model".to_string(), Value::String(model.to_string()));

    // OpenAI's `store: false`: do not keep this conversation (it means "no training").
    if compat.supports_store {
        body.insert("store".to_string(), Value::Bool(false));
    }

    // `prompt_cache_key`: the server groups one prefix's requests by it; the value is the session id.
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

    // ---- Does this request think? Three gates, in order of authority: (1) a level was asked for,
    // (2) the wire takes reasoning params, (3) no tool-choice rule forbids it (3 outranks 1).
    let wants_thinking = compat.reasoning_effort.is_some() && compat.supports_reasoning_params;
    let forces_tool = compat.raw_tool_choice.is_some()
        || matches!(compat.tool_choice.as_deref(), Some("required") | Some("any"));
    let has_tools = tools.as_array().is_some_and(|a| !a.is_empty());
    let thinking_forbidden_by_tools = (forces_tool && compat.disable_reasoning_on_forced_tool_choice)
        || (compat.tool_choice.is_some() && compat.disable_reasoning_on_tool_choice)
        || (has_tools && compat.disable_reasoning_with_tools);
    let thinking_active = wants_thinking && !thinking_forbidden_by_tools;

    // `when_thinking` overlays a second set of switches; taken by pointer, never spread.
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

    // ---- Tools ---- An empty roster omits the field: strict hosts reject `"tools": []`.
    if has_tools {
        let mut wire_tools = tools.clone();
        // `strict: true` is not just "add a field": it requires `additionalProperties: false` on every
        // object and every `properties` key in `required`; missing either is a 400.
        if compat.supports_strict_mode
            && compat.tool_strict_mode == crate::server::agent::request::assembly::compat::ToolStrictMode::AllStrict
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

    // `tool_choice` has three downgrade paths: the host takes no parameter, no forced value, or a string only.
    if compat.supports_tool_choice && has_tools {
        // With K3 native reasoning on, a named function is a 400: keep "must call", let the model choose.
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

    // GLM-5.2's `tool_stream`: required with tools, or the arguments do not stream back.
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

    // ---- Sampling ---- OpenAI's reasoning models (official, Azure, Copilot) answer 400 to `temperature`.
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

    // ---- Reasoning ---- One intent, a dozen spellings; when thinking is off some wires still need an
    // explicit off signal (otherwise the model thinks anyway, or a strict host 400s).
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
        // `thinking.keep` (Z.ai / Moonshot): keep past thinking in the context; only these dialects have a place for it.
        if let Some(keep) = &compat.thinking_keep
            && matches!(format, ThinkingFormat::Zai | ThinkingFormat::Kimi)
            && let Some(t) = body.get_mut("thinking").and_then(|v| v.as_object_mut())
        {
            t.insert("keep".to_string(), json!(keep));
        }

        // Qwen's chat-template kwarg: without it the template strips earlier `<think>` blocks and the cache dies.
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

        // Some hosts take `reasoning_effort` as a **separate** top-level field; `omit_reasoning_effort` says dialect only.
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
                // Not sent when a forced tool choice turns reasoning off on OpenRouter: it clashes with `tool_choice`.
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

    // The reasoning token cap: reasoning and the answer share `max_tokens`; sent only when the host names it.
    if thinking_active
        && let (Some(field), Some(budget)) = (
            compat.thinking_token_budget_field,
            compat.thinking_budget,
        )
    {
        body.insert(field.as_str().to_string(), json!(budget));
    }

    // Long cache retention: OpenAI's `prompt_cache_retention: "24h"`; only for hosts that take the field.
    if compat.supports_long_prompt_cache_retention {
        body.insert("prompt_cache_retention".to_string(), json!("24h"));
    }

    // Explicit cache breakpoint: mark the last **stable** input message (later is better, but not the
    // last one — its content changes next round).
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

    // ---- Escape hatch ---- Always last: what the user wrote wins over everything above.
    if let Some(extra) = &compat.extra_body {
        for (k, v) in extra {
            body.insert(k.clone(), v.clone());
        }
    }

    // `reasoning_effort` and `thinking` cannot coexist (some hosts 400); done after the escape hatch.
    if compat.drop_thinking_when_reasoning_effort && body.contains_key("reasoning_effort") {
        body.remove("thinking");
    }

    Value::Object(body)
}


/// Internal messages → the wire array, **per host**, deciding how replayed reasoning goes out: nothing by
/// default, opened switch by switch. Also fixes tool-result `name` and tool-call ids.
pub(crate) fn messages_to_wire(
    messages: &[crate::server::agent::request::message::WireMessage],
    compat: &crate::server::agent::request::assembly::compat::Resolved,
) -> serde_json::Value {
    use serde_json::{Value, json};

    // When a tool message needs `name`, it can only come from the assistant message that called it.
    let mut tool_names: std::collections::HashMap<&str, &str> = std::collections::HashMap::new();
    for m in messages {
        if let crate::server::agent::request::message::WireMessage::Assistant { tool_calls, .. } = m {
            for c in tool_calls {
                tool_names.insert(c.id.as_str(), c.function.name.as_str());
            }
        }
    }

    let mut out: Vec<Value> = Vec::with_capacity(messages.len());
    for m in messages {
        let mut v = serde_json::to_value(m).expect("Message serialization is infallible");
        let obj = v.as_object_mut().expect("Message serializes to an object");

        match m {
            crate::server::agent::request::message::WireMessage::Assistant {
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
            crate::server::agent::request::message::WireMessage::System { .. } => {
                // Some hosts call the system message `developer` (OpenAI's o-series); the wrong name is a 400.
                if compat.supports_developer_role
                    && obj.get("role").and_then(|r| r.as_str()) == Some("system")
                {
                    obj.insert("role".to_string(), json!("developer"));
                }
            }
            crate::server::agent::request::message::WireMessage::Tool { tool_call_id, .. } => {
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
        // OpenAI caps tool-call ids at 40 characters; truncation must hit `tool_calls[].id` and the tool message.
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

    // A strict chat template takes **one** leading system message; false joins the ordered prompts into
    // one (a slightly worse tail cache beats a rejection).
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

    // Some hosts refuse a user message right after a tool result: an assistant must sit between.
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

/// Two image repairs: a text-only host gets a placeholder line, and `detail: "original"`. Must run
/// **before** the cache breakpoint.
fn maybe_patch_images(
    messages: &mut [serde_json::Value],
    compat: &crate::server::agent::request::assembly::compat::Resolved,
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
                // A placeholder rather than deletion: an empty content array is a format error to some hosts.
                *part = json!({ "type": "text", "text": "[image omitted]" });
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

/// Anthropic's explicit breakpoint: a `cache_control` marker on the last user/assistant message with
/// content. Off by default — writing a cache costs extra, so only someone who knows why turns it on.
fn maybe_add_anthropic_cache_control(
    messages: &mut [serde_json::Value],
    compat: &crate::server::agent::request::assembly::compat::Resolved,
) {
    use crate::server::agent::request::assembly::compat::CacheControlFormat;
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
            // Structured (multimodal) content is left alone: an empty text block would be rejected.
            continue;
        };
        // An empty assistant turn (pure tool calls) must **not** become structured content.
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

/// The reasoning handling for one assistant message. See [`messages_to_wire`].
fn patch_assistant(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    reasoning: Option<&str>,
    has_tool_calls: bool,
    compat: &crate::server::agent::request::assembly::compat::Resolved,
) {
    use serde_json::json;

    // `reasoning` is the internal name; whether it reaches the wire, and as what, is decided below.
    obj.remove("reasoning");

    // A tool-call turn with null content: some hosts refuse `content: null`.
    if has_tool_calls && compat.requires_assistant_content_for_tool_calls {
        let cur = obj.get("content").and_then(|c| c.as_str()).unwrap_or("");
        obj.insert("content".to_string(), json!(cur));
    }

    let reasoning = reasoning.filter(|r| !r.is_empty());

    // Some hosts know no separate reasoning field and want thinking folded into content as text.
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
    let by_cache = compat.resend_reasoning_content;
    let by_dialect = compat.thinking_format == crate::server::agent::request::assembly::compat::ThinkingFormat::Zai;
    if !(by_tools || by_all || by_cache || by_dialect) {
        return;
    }
    // `by_tools` is needed on tool-call turns only; the other three are "every turn" wants.
    if !(by_all || by_cache || by_dialect) && !has_tool_calls {
        return;
    }

    let value = match reasoning {
        Some(r) => r.to_string(),
        // No reasoning content but the field is **required**: an empty string beats a host's validator
        // error. A placeholder like `.` only when the host says it takes one.
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

/// Rewrite a tool's argument schema into what OpenAI's strict mode accepts: `additionalProperties: false`
/// on every object, and every property in `required` (optionality is expressed as "T or null").
fn enforce_strict_schema(node: &mut serde_json::Value) {
    use serde_json::{Map, Value, json};

    let Value::Object(obj) = node else {
        return;
    };

    if obj.get("type").and_then(|t| t.as_str()) == Some("object") {
        obj.insert("additionalProperties".to_string(), Value::Bool(false));

        // Rewrite each property first, then decide which ones wrap as nullable.
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

        // `required` is written back after the properties, so collect the keys while the borrow is live.
        let all_keys: Vec<String> = match obj.get_mut("properties") {
            Some(Value::Object(props)) => {
                for (key, value) in props.iter_mut() {
                    enforce_strict_schema(value);
                    if originally_required.iter().any(|r| r == key) || already_nullable(value) {
                        continue;
                    }
                    // Optional → nullable. `description` stays outside the union, or hosts lose the manual.
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
            // Strict mode wants `required` complete and in a stable order: follow the properties' order.
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

/// Is this node already written as nullable? Avoids a second `anyOf` wrapper.
fn already_nullable(node: &serde_json::Value) -> bool {
    node.get("anyOf")
        .and_then(|a| a.as_array())
        .is_some_and(|branches| {
            branches
                .iter()
                .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("null"))
        })
}

/// Normalise a session id to what `prompt_cache_key` accepts (64 characters). Longer becomes a stable
/// hash, and it must be deterministic: a new key every round caches nothing.
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

/// Mark the last **stable** input message with an explicit cache breakpoint: the last user message is
/// this round's question, so walk back from it to the first cacheable one.
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

/// Normalise any tool-call id to Mistral's "exactly 9 alphanumerics". It must be deterministic, or the
/// assistant's `tool_calls[].id` no longer matches the following tool message.
fn mistral_tool_id(id: &str) -> String {
    let alnum: String = id.chars().filter(char::is_ascii_alphanumeric).collect();
    if alnum.len() == 9 {
        return alnum;
    }
    // FNV-1a: short, no dependency, stable across processes.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in id.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:09}", h % 1_000_000_000)
}
