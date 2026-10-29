//! `todo_off` — the todo tool is switched off for this session.

use anyhow::Result;
use serde_json::Value;

use crate::server::api::{Api, Refuse};

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    super::todo_on::switch(api, viewer, ask, false)
}
