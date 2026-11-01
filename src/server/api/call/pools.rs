//! `pools` — the models and the profiles a session may be opened with.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::{Api, Refuse};

pub(crate) fn answer(api: &Api) -> Result<Value, Refuse> {
    let cfg = api.cfg();
    let models: Vec<Value> = cfg
        .models()
        .map(|(provider, entry)| {
            json!({
                "provider": provider,
                "id": entry.id,
                "address": format!("{provider}:{}", entry.id),
                "name": entry.display_name(),
                "context_window": entry.context_window,
                "max_output_tokens": entry.max_output_tokens,
                "currency": entry.currency.as_str(),
            })
        })
        .collect();
    Ok(json!({"models": models, "profiles": cfg.profiles()}))
}
