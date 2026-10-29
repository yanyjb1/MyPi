//! Config queries: a registry-shaped value, resolved against the viewer's attached session for
//! `model.*` and `profile.*`.

use anyhow::{Context, Result, anyhow, bail};

use crate::server::api::call::ctx::Ctx;
use crate::server::config::Config;
use crate::server::config::read::profile;

use crate::ipc::contract::call::config::ConfigValue;

pub(crate) fn value(ctx: &Ctx, key: &str) -> Result<ConfigValue> {
    if key.starts_with("model.") {
        return model_value(ctx, key);
    }
    if key.starts_with("profile.") {
        return profile_value(ctx, key);
    }
    global_value(ctx.cfg, key)
}

fn model_value(ctx: &Ctx, key: &str) -> Result<ConfigValue> {
    let resolved = match ctx.attached {
        Some(session) => {
            let row: Option<(Option<String>, Option<String>)> = ctx
                .reader
                .query_row(
                    "SELECT model_provider, model FROM sessions WHERE id = ?1",
                    [session],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .ok();
            match row {
                Some((Some(provider), Some(model))) => {
                    ctx.cfg.model_by_id(&format!("{provider}:{model}"))?
                }
                _ => ctx.cfg.default_model()?,
            }
        }
        None => ctx.cfg.default_model()?,
    };
    let entry = &resolved.entry;
    Ok(match key {
        "model.id" => ConfigValue::Text(entry.id.clone()),
        "model.provider" => ConfigValue::Text(resolved.provider_name.clone()),
        "model.name" => ConfigValue::Text(entry.display_name().to_string()),
        "model.contextWindow" => ConfigValue::Int(entry.context_window as i64),
        "model.maxOutputTokens" => ConfigValue::Int(entry.max_output_tokens.unwrap_or(0) as i64),
        other => bail!("unknown config key: {other}"),
    })
}

fn profile_value(ctx: &Ctx, key: &str) -> Result<ConfigValue> {
    let stored = ctx.attached.and_then(|session| {
        ctx.reader
            .query_row(
                "SELECT profile FROM sessions WHERE id = ?1",
                [session],
                |r| r.get::<_, Option<String>>(0),
            )
            .ok()
            .flatten()
    });
    let name = stored.unwrap_or_else(|| profile::active_name(ctx.cfg));
    let resolved = profile::resolve(ctx.cfg, &name)?;
    Ok(match key {
        "profile.name" => ConfigValue::Text(name),
        "profile.system" => ConfigValue::Text(resolved.system),
        "profile.tools" => ConfigValue::List(
            resolved
                .tools
                .unwrap_or_default()
                .into_iter()
                .map(ConfigValue::Text)
                .collect(),
        ),
        "profile.compactInstruction" => ConfigValue::Text(resolved.compact_instruction),
        other => bail!("unknown config key: {other}"),
    })
}

fn global_value(cfg: &Config, key: &str) -> Result<ConfigValue> {
    let key = key.strip_prefix("config.").unwrap_or(key);
    let json = serde_json::to_value(&cfg.app).context("the config does not serialize")?;
    let mut current = &json;
    for part in key.split('.') {
        current = current
            .get(part)
            .ok_or_else(|| anyhow!("unknown config key: {key}"))?;
    }
    from_json(current)
}

fn from_json(value: &serde_json::Value) -> Result<ConfigValue> {
    Ok(match value {
        serde_json::Value::Bool(b) => ConfigValue::Bool(*b),
        serde_json::Value::Number(n) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => ConfigValue::Int(i),
            (None, Some(f)) => ConfigValue::Float(f),
            _ => bail!("a config number is neither an integer nor a float"),
        },
        serde_json::Value::String(s) => ConfigValue::Text(s.clone()),
        serde_json::Value::Array(items) => {
            ConfigValue::List(items.iter().map(from_json).collect::<Result<Vec<_>>>()?)
        }
        serde_json::Value::Null => ConfigValue::Text("null".to_string()),
        serde_json::Value::Object(_) => bail!("a config object is not a value"),
    })
}
