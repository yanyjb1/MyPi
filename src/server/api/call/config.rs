// `config` — one key's value, or a model's whole display entry.

use anyhow::Result;
use serde_json::{Value, json};

use crate::platform::{env, time};
use crate::server::api::{Api, Refuse};
use crate::server::config::{
    KEY_CONFIG_SERVER_API_DEFAULT_MODEL, KEY_CONFIG_SERVER_PLATFORM_HOME,
    KEY_CONFIG_SERVER_PLATFORM_TIMEZONE, ResolvedModel,
};

// A keyed answer: the caller never has to remember what it asked for.
pub(crate) fn answer(api: &Api, key: &str) -> Result<Value, Refuse> {
    Ok(json!({"key": key, "value": value(api, key)?}))
}

fn value(api: &Api, key: &str) -> Result<Value, Refuse> {
    let cfg = api.cfg();
    // The offset is answered normalized: the machine's own when the key is unset or a name.
    if key == KEY_CONFIG_SERVER_PLATFORM_TIMEZONE {
        return Ok(Value::String(time::display()));
    }
    // What `~` stands for is answered resolved too: the platform's own home when the key is unset.
    if key == KEY_CONFIG_SERVER_PLATFORM_HOME {
        return Ok(env::tilde_home().map_or(Value::Null, |home| {
            Value::String(home.to_string_lossy().into_owned())
        }));
    }
    // The default model is answered resolved: an unset key still names the table's fallback.
    if key == KEY_CONFIG_SERVER_API_DEFAULT_MODEL {
        // This call asks before any session exists: the process's own directory answers.
        let cwd = std::env::current_dir().unwrap_or_default();
        let address = cfg
            .default_model(None, &cwd)
            .map(|model| format!("{}:{}", model.provider_name, model.entry.id));
        return Ok(address.map(Value::String).unwrap_or(Value::Null));
    }
    // A bare model address answers the whole entry: one round trip per model change.
    if let Some(address) = key.strip_prefix("models.model.")
        && let Some(model) = cfg.model_by_id(address)
    {
        return Ok(entry(&model));
    }
    cfg.get_json(key, None)
        .map_err(|_| Refuse::new("config_key_unknown").args(json!({"key": key})))
}

// Everything a front end draws for one model.
fn entry(model: &ResolvedModel) -> Value {
    let entry = &model.entry;
    json!({
        "provider": model.provider_name,
        "address": format!("{}:{}", model.provider_name, entry.id),
        "id": entry.id,
        "name": entry.display_name(),
        "contextWindow": entry.context_window,
        "maxOutputTokens": entry.max_output_tokens,
        "vision": entry.vision,
        "reasoningEfforts": entry
            .efforts
            .iter()
            .map(|rung| rung.as_str())
            .collect::<Vec<_>>(),
        "currency": entry.currency.as_str(),
        "cost": {
            "input": entry.cost.input,
            "output": entry.cost.output,
            "cacheRead": entry.cost.cache_read,
            "cacheWrite": entry.cost.cache_write,
        },
    })
}
