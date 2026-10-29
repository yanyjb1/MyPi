//! `pools` — not wired: the candidates come from the config.

use anyhow::Result;
use serde_json::Value;

use crate::server::api::{Api, Refuse};

pub(crate) fn answer(_api: &Api) -> Result<Value, Refuse> {
    Err(Refuse::Internal("the config is not wired yet".into()))
}
