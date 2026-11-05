//! `config` — one key's value, read through the domain that owns it.

use anyhow::Result;
use serde_json::Value;

use crate::server::api::{Api, Refuse};
use crate::server::config::KEY_CONFIG_SERVER_API_DEFAULT_MODEL;

pub(crate) fn answer(api: &Api, key: &str) -> Result<Value, Refuse> {
    let cfg = api.cfg();
    // The default model is answered resolved: an unset key still names the table's fallback.
    if key == KEY_CONFIG_SERVER_API_DEFAULT_MODEL {
        let address = cfg
            .default_model(None)
            .map(|model| format!("{}:{}", model.provider_name, model.entry.id));
        return Ok(address.map(Value::String).unwrap_or(Value::Null));
    }
    cfg.get_json(key, None).map_err(Refuse::Bad)
}
