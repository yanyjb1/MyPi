//! `config` — not wired: the config lives outside this region.

use anyhow::Result;
use serde_json::Value;

use crate::server::api::{Api, Refuse};

pub(crate) fn answer(_api: &Api, _key: &str) -> Result<Value, Refuse> {
    Err(Refuse::Internal("the config is not wired yet".into()))
}
