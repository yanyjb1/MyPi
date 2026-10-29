//! `cwd` — the working directory, persisted on the session.

use anyhow::{Context, Result};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::server::api::route::{i64_of, str_of};
use crate::server::api::{Api, Refuse};
use crate::server::storage::{SessionChange, Submit};

use super::emit;

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    let raw = str_of(ask, "path")?;
    emit::require_writer(api, viewer, session)?;

    let base = emit::stored_cwd(api, session)?
        .map(PathBuf::from)
        .unwrap_or_default();
    let target = resolve(&base, raw)?;
    let turn = emit::tip_turn(api, session)?;
    api.writer().push(Submit::Session {
        session,
        change: SessionChange::Cwd {
            turn,
            path: target.display().to_string(),
        },
    })?;
    emit::changed(api, viewer, session, "cwd")?;
    Ok(serde_json::json!({}))
}

fn resolve(cwd: &Path, raw: &str) -> Result<PathBuf> {
    let raw = raw.trim();
    anyhow::ensure!(!raw.is_empty(), "usage: /cdp <dir>");
    let path = match raw.strip_prefix('~') {
        Some(rest) => crate::server::platform::env::home()
            .unwrap_or_default()
            .join(rest),
        None => PathBuf::from(raw),
    };
    let path = if path.is_absolute() {
        path
    } else {
        cwd.join(path)
    };
    let real = path
        .canonicalize()
        .with_context(|| format!("no such directory: {raw}"))?;
    anyhow::ensure!(real.is_dir(), "not a directory: {raw}");
    Ok(real)
}
