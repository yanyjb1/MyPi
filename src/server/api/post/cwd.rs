//! `cwd` — the working directory: resolved and licensed here, announced once it lands.

use std::path::PathBuf;

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::agent::r#loop::{Change, NoticeLevel};
use crate::server::api::route::{i64_of, str_of};
use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;

use super::emit;

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    let path = str_of(ask, "path")?;
    emit::require_writer(api, viewer, session)?;
    api.ensure_booked(session)?;
    if let Err(e) = api
        .ledger()
        .change(session, Change::Cwd(PathBuf::from(path)))
    {
        let text = format!("{e:#}");
        api.relay()
            .notice(session, NoticeLevel::Error, text.clone());
        return Err(Refuse::Bad(text));
    }
    emit::changed(api, viewer, session, "cwd")?;
    let cwd = Reader::open(api.db())?
        .session(session)?
        .and_then(|row| row["cwd"].as_str().map(str::to_string))
        .unwrap_or_default();
    Ok(json!({"cwd": cwd}))
}
