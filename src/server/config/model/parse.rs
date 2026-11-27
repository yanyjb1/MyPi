use anyhow::{Context as _, bail};
use serde_json::json;
use serde_yaml::{Mapping, Value as Yaml};

use super::super::notice::{ConfigNotice, warn, warn_args};
use super::super::registry;
use super::protocol::ProtocolId;
use super::types::{ModelEntry, ModelsConfig, Provider};

use super::entry::parse_model;
const TOP_KEYS: &[&str] = &["providers"];

pub fn parse(text: &str, file: &str) -> anyhow::Result<(ModelsConfig, Vec<ConfigNotice>)> {
    let root: Yaml =
        serde_yaml::from_str(text).with_context(|| format!("invalid models.yml: {file}"))?;
    let top = root
        .as_mapping()
        .ok_or_else(|| anyhow::anyhow!("models.yml must be a mapping: {file}"))?;
    let mut notices = Vec::new();
    unknown_keys(top, TOP_KEYS, "models", file, &mut notices);
    let providers_raw = top
        .get("providers")
        .and_then(Yaml::as_mapping)
        .ok_or_else(|| anyhow::anyhow!("models.yml has no `providers:` mapping: {file}"))?;

    let mut providers = Vec::new();
    for (name, raw) in providers_raw {
        let Some(name) = name.as_str() else {
            notices.push(warn(
                "models.providers",
                file,
                "config_provider_name_not_text",
                "skipped",
            ));
            continue;
        };
        let key = format!("models.provider.{name}");
        if !valid_name(name) {
            notices.push(warn(&key, file, "config_provider_name_bad", "skipped"));
            continue;
        }
        let Some(map) = raw.as_mapping() else {
            notices.push(warn(&key, file, "config_provider_not_mapping", "skipped"));
            continue;
        };
        let allowed: Vec<&str> = registry::provider_fields().collect();
        unknown_keys(map, &allowed, &key, file, &mut notices);
        let base_url = match text_field(map, "baseUrl") {
            Some(v) if v.starts_with("http://") || v.starts_with("https://") => v.to_string(),
            Some(_) => {
                notices.push(warn(&key, file, "config_base_url_bad", "provider skipped"));
                continue;
            }
            None => {
                notices.push(warn(
                    &key,
                    file,
                    "config_base_url_missing",
                    "provider skipped",
                ));
                continue;
            }
        };
        let protocol = match map.get("api") {
            None => {
                notices.push(warn(&key, file, "config_api_missing", "provider skipped"));
                continue;
            }
            Some(value) => match value.as_str() {
                Some(raw) => match ProtocolId::parse(raw) {
                    Ok(id) => id,
                    Err(_) => {
                        notices.push(warn_args(
                            &key,
                            file,
                            "config_api_unknown",
                            "provider skipped",
                            json!({"api": raw}),
                        ));
                        continue;
                    }
                },
                None => {
                    notices.push(warn(&key, file, "config_api_not_text", "provider skipped"));
                    continue;
                }
            },
        };
        let api_key = match map.get("apiKey") {
            None => String::new(),
            Some(value) => match value.as_str() {
                Some(text) => text.to_string(),
                None => {
                    notices.push(warn(&key, file, "config_api_key_not_text", "empty"));
                    String::new()
                }
            },
        };
        let auth_header = match map.get("authHeader") {
            None => true,
            Some(v) => match v.as_bool() {
                Some(b) => b,
                None => {
                    notices.push(warn(&key, file, "config_auth_header_bad", "true"));
                    true
                }
            },
        };
        let Some(list) = map.get("models").and_then(Yaml::as_sequence) else {
            notices.push(warn(
                &key,
                file,
                "config_models_not_list",
                "provider skipped",
            ));
            continue;
        };
        if list.is_empty() {
            notices.push(warn(&key, file, "config_models_empty", "provider skipped"));
            continue;
        }
        let mut models: Vec<ModelEntry> = Vec::new();
        for entry in list {
            if entry.as_mapping().is_none() {
                notices.push(warn(
                    &key,
                    file,
                    "config_model_not_mapping",
                    "entry skipped",
                ));
                continue;
            }
            match parse_model(entry, &key, file, &mut notices) {
                Some(model) => {
                    if models.iter().any(|m| m.id == model.id) {
                        notices.push(warn(
                            &format!("{key}.models.{}", model.id),
                            file,
                            "config_model_duplicate",
                            "entry skipped",
                        ));
                    } else {
                        models.push(model);
                    }
                }
                None => {}
            }
        }
        if models.is_empty() {
            notices.push(warn(&key, file, "config_model_none", "provider skipped"));
            continue;
        }
        providers.push(Provider {
            name: name.to_string(),
            base_url,
            protocol,
            api_key,
            auth_header,
            models,
        });
    }

    if providers.is_empty() {
        bail!("models.yml has no usable provider: {file}");
    }
    Ok((ModelsConfig { providers }, notices))
}

fn text_field<'a>(map: &'a Mapping, field: &str) -> Option<&'a str> {
    map.get(field).and_then(Yaml::as_str)
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('.')
        && !name.contains(':')
        && !name.contains(char::is_whitespace)
}

pub(super) fn unknown_keys(
    map: &Mapping,
    allowed: &[&str],
    base: &str,
    file: &str,
    notices: &mut Vec<ConfigNotice>,
) {
    for key in map.keys() {
        let Some(name) = key.as_str() else { continue };
        if !allowed.contains(&name) {
            notices.push(warn(
                &format!("{base}.{name}"),
                file,
                "config_unknown_key",
                "ignored",
            ));
        }
    }
}
