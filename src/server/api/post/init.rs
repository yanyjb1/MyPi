// `init` — the session row, its request archive, and the loop's booking.

use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::agent::r#loop::Birth;
use crate::server::agent::tools::registry;
use crate::server::agent::tools::subagent::FINAL;
use crate::server::api::route::{opt_str_of, str_of};
use crate::server::api::{Api, Refuse};
use crate::server::storage::{Ref, RequestMeta, Row, Submit};

use super::emit;

pub(crate) fn apply(api: &Api, ask: &Value) -> Result<Value, Refuse> {
    // The launch directory exists; only where it may stand is asked, and a barred one lands in the temp dir.
    let cwd = crate::server::agent::tools::workspace::launch_dir(str_of(ask, "cwd")?);
    let name = opt_str_of(ask, "name")?.map(str::to_string);
    let profile = opt_str_of(ask, "profile")?.map(str::to_string);
    let stated = match ask.get("model") {
        Some(value) if !value.is_null() => Some(emit::model_of(value)?),
        _ => None,
    };

    let cfg = api.cfg();
    let resolved = cfg.resolve_profile(profile.as_deref(), &cwd)?;
    let (provider, model) = match stated {
        Some(pair) => pair,
        None => {
            let default = cfg
                .default_model(profile.as_deref(), &cwd)
                .ok_or_else(|| Refuse::new("no_default_model"))?;
            (default.provider_name, default.entry.id)
        }
    };
    // A main session never resolves the child's `final`: no name, no manual, whatever the profile lists.
    let tools: Vec<String> = resolved
        .tools
        .unwrap_or_else(registry::defaults)
        .into_iter()
        .filter(|name| name != FINAL.name)
        .collect();
    let manuals = Arc::new(registry::roster(&tools));
    let tools_json = serde_json::to_string(&*manuals).map_err(|_| Refuse::new("internal"))?;

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
        return Err(Refuse::new("no_session_row"));
    }

    let vision = api
        .cfg()
        .model_by_id(&format!("{provider}:{model}"))
        .map(|resolved| resolved.entry.vision)
        .unwrap_or(false);
    api.ledger().book(Birth {
        session,
        provider,
        model,
        system,
        tools,
        manuals,
        compact: resolved.compact_instruction,
        effort: crate::server::agent::request::Effort::Auto,
        vision,
        meta: Some(archive),
        cwd,
    })?;
    Ok(json!({"session": session}))
}
