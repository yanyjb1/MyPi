//! One model entry, parsed with warnings.

use serde_yaml::{Mapping, Value as Yaml};

use super::super::notice::{ConfigNotice, warn};
use super::super::registry;
use super::parse::unknown_keys;
use super::types::{Cost, Currency, ModelEntry};

pub(super) fn parse_model(
    raw: &Yaml,
    base: &str,
    file: &str,
    notices: &mut Vec<ConfigNotice>,
) -> Option<ModelEntry> {
    let map = raw.as_mapping()?;
    let key = format!("{base}.models");
    let allowed: Vec<&str> = registry::model_fields().collect();
    unknown_keys(map, &allowed, &key, file, notices);
    let id = match map.get("id") {
        None => {
            notices.push(warn(&key, file, "id is required", "entry skipped"));
            return None;
        }
        Some(value) => match value.as_str() {
            Some(id) if !id.is_empty() => id.to_string(),
            Some(_) => {
                notices.push(warn(&key, file, "id must not be empty", "entry skipped"));
                return None;
            }
            None => {
                notices.push(warn(&key, file, "id must be text", "entry skipped"));
                return None;
            }
        },
    };
    let name = match map.get("name") {
        None => String::new(),
        Some(value) => match value.as_str() {
            Some(name) => name.to_string(),
            None => {
                notices.push(warn(&key, file, "name must be text", "id"));
                String::new()
            }
        },
    };
    let context_window = optional_u64(map, "contextWindow", &key, file, notices);
    let max_output_tokens = optional_u64(map, "maxOutputTokens", &key, file, notices);
    let currency = match map.get("currency").and_then(Yaml::as_str) {
        None => Currency::CNY,
        Some(raw) => match Currency::parse(raw) {
            Some(currency) => currency,
            None => {
                notices.push(warn(
                    &key,
                    file,
                    &format!("unknown currency `{raw}`"),
                    "CNY",
                ));
                Currency::CNY
            }
        },
    };
    let cost = match map.get("cost") {
        None => Cost::default(),
        Some(raw) => parse_cost(raw, &key, file, notices),
    };
    Some(ModelEntry {
        id,
        name,
        context_window,
        max_output_tokens,
        currency,
        cost,
    })
}

fn parse_cost(raw: &Yaml, base: &str, file: &str, notices: &mut Vec<ConfigNotice>) -> Cost {
    let Some(map) = raw.as_mapping() else {
        notices.push(warn(base, file, "cost must be a mapping", "zero prices"));
        return Cost::default();
    };
    let allowed: Vec<&str> = registry::cost_fields().collect();
    unknown_keys(map, &allowed, &format!("{base}.cost"), file, notices);
    let mut cost = Cost::default();
    for (field, slot) in [
        ("input", &mut cost.input),
        ("output", &mut cost.output),
        ("cacheRead", &mut cost.cache_read),
        ("cacheWrite", &mut cost.cache_write),
    ] {
        let Some(value) = map.get(field) else {
            continue;
        };
        match value.as_f64() {
            Some(number) if number >= 0.0 => *slot = number,
            Some(_) => {
                notices.push(warn(
                    base,
                    file,
                    &format!("{field} must not be negative"),
                    "0",
                ));
            }
            None => {
                notices.push(warn(base, file, &format!("{field} must be a number"), "0"));
            }
        }
    }
    cost
}

fn optional_u64(
    map: &Mapping,
    field: &str,
    key: &str,
    file: &str,
    notices: &mut Vec<ConfigNotice>,
) -> Option<u64> {
    match map.get(field) {
        None | Some(Yaml::Null) => None,
        Some(value) => match value.as_u64() {
            Some(number) => Some(number),
            None => {
                notices.push(warn(
                    key,
                    file,
                    &format!("{field} must be a non-negative integer"),
                    "absent",
                ));
                None
            }
        },
    }
}
