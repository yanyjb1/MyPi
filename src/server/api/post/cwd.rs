//! `cwd` — the working directory: resolved and licensed here, answered as it landed.

use std::path::PathBuf;

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::agent::r#loop::{Change, NoticeLevel};
use crate::server::api::route::{i64_of, str_of};
use crate::server::api::{Api, Refuse};

use super::emit;

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    let path = str_of(ask, "path")?;
    emit::require_writer(api, viewer, session)?;
    api.ensure_booked(session)?;
    let landed = match api
        .ledger()
        .change(session, Change::Cwd(PathBuf::from(path)))
    {
        Ok(landed) => landed,
        Err(e) => {
            let text = format!("{e:#}");
            api.relay().notice(session, NoticeLevel::Error, text.clone());
            return Err(Refuse::Bad(text));
        }
    };
    emit::changed(api, viewer, session, "cwd")?;
    let cwd = landed
        .map(|real| real.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(json!({ "cwd": cwd }))
}
