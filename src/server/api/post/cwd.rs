// `cwd` — the working directory: resolved and licensed here, answered as it landed.

use std::path::PathBuf;

use anyhow::Result;
use serde_json::{Value, json};

use crate::ipc::contract::Text;
use crate::server::agent::r#loop::Change;
use crate::server::api::route::{i64_of, str_of};
use crate::server::api::{Api, Refuse};

use super::emit;

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    let path = str_of(ask, "path")?;
    emit::prepare(api, viewer, session)?;
    let landed = match api
        .ledger()
        .change(session, Change::Cwd(PathBuf::from(path)))
    {
        Ok(landed) => landed,
        Err(_) => {
            let text = Text::error("cwd_unusable").args(json!({"path": path}));
            api.relay().notice(session, text.clone());
            return Err(Refuse(text));
        }
    };
    let cwd = landed
        .map(|real| real.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(json!({ "cwd": cwd }))
}
