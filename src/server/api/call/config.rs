//! `config` — one key's value, read through the domain that owns it.

use anyhow::Result;
use serde_json::Value;

use crate::server::api::{Api, Refuse};

pub(crate) fn answer(api: &Api, key: &str) -> Result<Value, Refuse> {
    let cfg = api.cfg();
    cfg.get_json(key, None).map_err(Refuse::Bad)
}
