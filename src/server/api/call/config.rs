//! `config` — one key's value, or a model's whole display entry.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::{Api, Refuse};
use crate::server::config::{KEY_CONFIG_SERVER_API_DEFAULT_MODEL, ResolvedModel};

pub(crate) fn answer(api: &Api, key: &str) -> Result<Value, Refuse> {
    let cfg = api.cfg();
    // The default model is answered resolved: an unset key still names the table's fallback.
    if key == KEY_CONFIG_SERVER_API_DEFAULT_MODEL {
        // This door asks before any session exists: the process's own directory answers.
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
    cfg.get_json(key, None).map_err(Refuse::Bad)
}

/// Everything a front end draws for one model.
fn entry(model: &ResolvedModel) -> Value {
    let entry = &model.entry;
    json!({
        "provider": model.provider_name,
        "address": format!("{}:{}", model.provider_name, entry.id),
        "id": entry.id,
        "name": entry.display_name(),
        "contextWindow": entry.context_window,
        "maxOutputTokens": entry.max_output_tokens,
        "currency": entry.currency.as_str(),
        "cost": {
            "input": entry.cost.input,
            "output": entry.cost.output,
            "cacheRead": entry.cost.cache_read,
            "cacheWrite": entry.cost.cache_write,
        },
    })
}
