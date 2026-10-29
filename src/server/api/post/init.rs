//! `init` — a session row before any message; the front end needs the id first.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::route::{opt_str_of, str_of};
use crate::server::api::{Api, Refuse};
use crate::server::storage::{SessionChange, Submit};

use super::emit;

pub(crate) fn apply(api: &Api, _conn: u64, _viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let cwd = str_of(ask, "cwd")?.to_string();
    let name = opt_str_of(ask, "name")?.map(str::to_string);
    let profile = opt_str_of(ask, "profile")?.map(str::to_string);
    let model = match ask.get("model") {
        Some(value) if !value.is_null() => Some(emit::model_of(value)?),
        _ => None,
    };

    let landed = api.writer().push(Submit::NewSession {
        started_at: crate::server::platform::time::now_stamp(),
        cwd,
        model,
        name,
        profile: profile.clone(),
    })?;
    let session = landed
        .session
        .ok_or_else(|| Refuse::Internal("the writer created no session row".into()))?;

    if let Some(profile) = profile {
        api.writer().push(Submit::Session {
            session,
            change: SessionChange::Profile(profile.clone()),
        })?;
        api.register().set(session, "profile", &profile)?;
    }
    Ok(json!({"session": session}))
}
