//! `init` — the session row, its request archive, and the loop's booking.

use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::agent::r#loop::Birth;
use crate::server::agent::tools::registry;
use crate::server::api::route::{opt_str_of, str_of};
use crate::server::api::{Api, Refuse};
use crate::server::storage::{Ref, RequestMeta, Row, Submit};

use super::emit;

pub(crate) fn apply(api: &Api, _conn: u64, _viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    // The launch directory exists; only where it may stand is asked, and a barred one lands in
    // the temp dir.
    let cwd = crate::server::agent::tools::workspace::launch_dir(str_of(ask, "cwd")?);
    let name = opt_str_of(ask, "name")?.map(str::to_string);
    let profile = opt_str_of(ask, "profile")?.map(str::to_string);
    let stated = match ask.get("model") {
        Some(value) if !value.is_null() => Some(emit::model_of(value)?),
        _ => None,
    };

    let cfg = api.cfg();
    let resolved = cfg.resolve_profile(profile.as_deref())?;
    let (provider, model) = match stated {
        Some(pair) => pair,
        None => {
            let default = cfg
                .default_model(profile.as_deref())
                .ok_or_else(|| Refuse::Internal("no default model is configured".into()))?;
            (default.provider_name, default.entry.id)
        }
    };
    let tools = resolved.tools.unwrap_or_else(registry::names);
    let manuals = Arc::new(registry::roster(&tools));
    let tools_json =
        serde_json::to_string(&*manuals).map_err(|e| Refuse::Internal(e.to_string()))?;

    let system = resolved.system;
    let landed = api.writer().push(Submit {
        rows: vec![
            Row::Request {
                meta: RequestMeta {
                    model: model.clone(),
                    provider: provider.clone(),
                    system: system.clone(),
                    tools_json,
                    baseline_prompt: None,
                },
            },
            Row::Session {
                started_at: crate::server::platform::time::now_stamp(),
                cwd: cwd.to_string_lossy().into_owned(),
                model: Some((provider.clone(), model.clone())),
                name,
                anchor: None,
                meta: Some(Ref::Batch(0)),
            },
        ],
    })?;
    let archive = landed.ids.first().copied().unwrap_or(0);
    let session = landed.ids.get(1).copied().unwrap_or(0);
    if session == 0 {
        return Err(Refuse::Internal("the writer created no session row".into()));
    }

    api.ledger().book(Birth {
        session,
        provider,
        model,
        system,
        tools,
        manuals,
        compact: resolved.compact_instruction,
        meta: Some(archive),
        cwd,
    })?;
    Ok(json!({"session": session}))
}
