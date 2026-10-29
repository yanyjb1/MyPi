//! `default_model` — not wired: the config lives outside this region.

use anyhow::Result;
use serde_json::Value;

use crate::server::api::{Api, Refuse};

pub(crate) fn apply(_api: &Api, _ask: &Value) -> Result<Value, Refuse> {
    Err(Refuse::Internal("the config is not wired yet".into()))
}
