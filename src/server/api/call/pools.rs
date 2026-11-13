//! `pools` — the models and the profiles a session may be opened with.

use std::path::PathBuf;

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::route::opt_str_of;
use crate::server::api::{Api, Refuse};

pub(crate) fn answer(api: &Api, ask: &Value) -> Result<Value, Refuse> {
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
    let cwd = cwd_of(api, ask)?;
    Ok(json!({"models": models, "profiles": cfg.profiles(&cwd)}))
}

/// Where the ask stands: the directory it names, else the attached session's, else this process's.
fn cwd_of(api: &Api, ask: &Value) -> Result<PathBuf, Refuse> {
    if let Some(path) = opt_str_of(ask, "cwd")?.filter(|path| !path.trim().is_empty()) {
        return Ok(PathBuf::from(path));
    }
    if let Some(session) = ask.get("session").and_then(Value::as_i64)
        && let Some(cwd) = api.ledger().cwd(session)
    {
        return Ok(cwd);
    }
    Ok(std::env::current_dir().unwrap_or_default())
}
