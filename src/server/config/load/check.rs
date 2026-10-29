use std::collections::{HashMap, HashSet};

use serde_yaml::Mapping;

use super::super::model::ModelsConfig;
use super::super::notice::{ConfigNotice, warn};
use super::super::registry::{self, Value};
use super::super::schema::{
    IPC_KINDS, KEY_CONFIG_SERVER_API_DEFAULT_MODEL, KEY_CONFIG_SERVER_PLATFORM_IPC,
    KEY_CONFIG_SERVER_PRUNE_ON_REWIND, KEY_CONFIG_SERVER_REQUEST_STREAMING,
    KEY_CONFIG_SERVER_STORAGE_BATCH_TOOLS, KEY_CONFIG_TUI_USAGE_ALIGN, STREAMING, USAGE_ALIGN,
};

pub(super) fn walk_unknown(
    map: &Mapping,
    prefix: &str,
    file: &str,
    notices: &mut Vec<ConfigNotice>,
    warned: &mut HashSet<String>,
) {
    for (key, value) in map {
        let Some(name) = key.as_str() else { continue };
        let full = format!("{prefix}.{name}");
        if registry::is_config_key(&full) {
            continue;
        }
        if registry::is_config_prefix(&full) {
            match value.as_mapping() {
                Some(sub) => walk_unknown(sub, &full, file, notices, warned),
                None => {
                    if warned.insert(full.clone()) {
                        notices.push(warn(&full, file, "expected a mapping", "ignored"));
                    }
                }
            }
        } else if warned.insert(full.clone()) {
            notices.push(warn(&full, file, "unknown key", "ignored"));
        }
    }
}

pub(super) fn check(
    values: &mut HashMap<&'static str, Value>,
    notices: &mut Vec<ConfigNotice>,
    models: &ModelsConfig,
) {
    if matches!(
        values.get(KEY_CONFIG_SERVER_PRUNE_ON_REWIND),
        Some(Value::Bool(false))
    ) {
        panic!("config.server.pruneOnRewind can only be true; the branch path is not built");
    }
    if matches!(
        values.get(KEY_CONFIG_SERVER_STORAGE_BATCH_TOOLS),
        Some(Value::Int(n)) if *n <= 0
    ) {
        let fallback = registry::lookup(KEY_CONFIG_SERVER_STORAGE_BATCH_TOOLS)
            .map(|key| key.default_value())
            .unwrap_or(Value::Null);
        notices.push(warn(
            KEY_CONFIG_SERVER_STORAGE_BATCH_TOOLS,
            "(merged)",
            "must be positive",
            "default",
        ));
        values.insert(KEY_CONFIG_SERVER_STORAGE_BATCH_TOOLS, fallback);
    }
    if let Some(Value::Text(mode)) = values.get(KEY_CONFIG_SERVER_REQUEST_STREAMING).cloned()
        && !STREAMING.contains(&mode.as_str())
    {
        notices.push(warn(
            KEY_CONFIG_SERVER_REQUEST_STREAMING,
            "(merged)",
            "unknown mode",
            STREAMING[0],
        ));
        values.insert(
            KEY_CONFIG_SERVER_REQUEST_STREAMING,
            Value::Text(STREAMING[0].to_string()),
        );
    }
    if let Some(Value::Text(kind)) = values.get(KEY_CONFIG_SERVER_PLATFORM_IPC).cloned()
        && !IPC_KINDS.contains(&kind.as_str())
    {
        notices.push(warn(
            KEY_CONFIG_SERVER_PLATFORM_IPC,
            "(merged)",
            "unknown endpoint kind",
            IPC_KINDS[0],
        ));
        values.insert(
            KEY_CONFIG_SERVER_PLATFORM_IPC,
            Value::Text(IPC_KINDS[0].to_string()),
        );
    }
    if let Some(Value::Text(align)) = values.get(KEY_CONFIG_TUI_USAGE_ALIGN).cloned()
        && !USAGE_ALIGN.contains(&align.as_str())
    {
        notices.push(warn(
            KEY_CONFIG_TUI_USAGE_ALIGN,
            "(merged)",
            "unknown alignment",
            USAGE_ALIGN[0],
        ));
        values.insert(
            KEY_CONFIG_TUI_USAGE_ALIGN,
            Value::Text(USAGE_ALIGN[0].to_string()),
        );
    }
    if let Some(Value::Text(address)) = values.get(KEY_CONFIG_SERVER_API_DEFAULT_MODEL).cloned()
        && models.model_by_id(&address).is_none()
    {
        let fallback = models
            .alphabetical_default()
            .map(|m| format!("{}:{}", m.provider_name, m.entry.id));
        notices.push(warn(
            KEY_CONFIG_SERVER_API_DEFAULT_MODEL,
            "(merged)",
            "the model does not resolve",
            fallback.as_deref().unwrap_or("none"),
        ));
        match fallback {
            Some(address) => {
                values.insert(KEY_CONFIG_SERVER_API_DEFAULT_MODEL, Value::Text(address));
            }
            None => {
                values.insert(KEY_CONFIG_SERVER_API_DEFAULT_MODEL, Value::Null);
            }
        }
    }
}
