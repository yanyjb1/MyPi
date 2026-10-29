//! `reload` — re-read the config; the config is not wired yet.

use anyhow::Result;
use serde_json::Value;

use crate::server::api::{Api, Refuse};

pub(crate) fn apply(_api: &Api) -> Result<Value, Refuse> {
    Err(Refuse::Internal("the config is not wired yet".into()))
}
