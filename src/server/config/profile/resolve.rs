use std::path::Path;

use anyhow::{Context as _, bail};
use serde_json::json;

use super::super::notice::{ConfigNotice, warn, warn_args};
use super::super::paths::{self, ConfigPaths};
use super::super::prompts;
use super::discover::{available, collect};
use super::types::{ResolvedProfile, builtin};

pub fn resolve(
    paths: &ConfigPaths,
    name: &str,
    compact_file: Option<&Path>,
    notices: &mut Vec<ConfigNotice>,
) -> anyhow::Result<ResolvedProfile> {
    let (found, unusable) = collect(paths, notices);
    if unusable.contains(name) {
        bail!("profile `{name}` has more than one directory in one root");
    }
    if let Some(dir) = found.get(name) {
        match load_dir(dir, compact_file, notices) {
            Ok(profile) => return Ok(profile),
            Err(error) if builtin(name).is_some() => {
                notices.push(warn_args(
                    &format!("profile.{name}"),
                    &dir.display().to_string(),
                    "config_profile_unreadable",
                    "compiled copy",
                    json!({"detail": error.to_string()}),
                ));
            }
            Err(error) => return Err(error),
        }
    }
    if let Some(compiled) = builtin(name) {
        return Ok(ResolvedProfile {
            system: (compiled.system)().to_string(),
            tools: None,
            compact_instruction: instruction(None, compact_file, (compiled.compact)(), notices),
        });
    }
    bail!(
        "unknown profile: {name}. Available: {}",
        available(found.keys().cloned()).join(", ")
    );
}

fn load_dir(
    dir: &Path,
    compact_file: Option<&Path>,
    notices: &mut Vec<ConfigNotice>,
) -> anyhow::Result<ResolvedProfile> {
    let system_path = dir.join(paths::SYSTEM_FILE);
    let system = std::fs::read_to_string(&system_path)
        .with_context(|| format!("failed to read {}", system_path.display()))?;
    let tools_path = dir.join(paths::TOOLS_FILE);
    let tools = if tools_path.is_file() {
        let names = load_tools(&tools_path, notices);
        (!names.is_empty()).then_some(names)
    } else {
        None
    };
    let own = dir.join(paths::COMPACT_FILE);
    let compact = if own.is_file() { Some(own) } else { None };
    Ok(ResolvedProfile {
        system,
        tools,
        compact_instruction: instruction(
            compact.as_deref(),
            compact_file,
            prompts::compaction(),
            notices,
        ),
    })
}

/// The compaction instruction: the profile's own file, then the configured one, then `fallback`.
fn instruction(
    profile: Option<&Path>,
    configured: Option<&Path>,
    fallback: &'static str,
    notices: &mut Vec<ConfigNotice>,
) -> String {
    if let Some(path) = profile {
        match std::fs::read_to_string(path) {
            Ok(text) => return text,
            Err(e) => notices.push(warn_args(
                "profile.compactInstruction",
                &path.display().to_string(),
                "config_instruction_unreadable",
                "compiled copy",
                json!({"detail": e.to_string()}),
            )),
        }
    }
    if let Some(path) = configured {
        match std::fs::read_to_string(path) {
            Ok(text) => return text,
            Err(e) => notices.push(warn_args(
                "config.server.request.compact.instructionFile",
                &path.display().to_string(),
                "config_instruction_unreadable",
                "compiled copy",
                json!({"detail": e.to_string()}),
            )),
        }
    }
    fallback.to_string()
}

fn load_tools(path: &Path, notices: &mut Vec<ConfigNotice>) -> Vec<String> {
    let file = path.display().to_string();
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            notices.push(warn_args(
                "profile.tools",
                &file,
                "config_tools_unreadable",
                "all tools",
                json!({"detail": error.to_string()}),
            ));
            return Vec::new();
        }
    };
    let value: serde_yaml::Value = match serde_yaml::from_str(&text) {
        Ok(value) => value,
        Err(error) => {
            notices.push(warn_args(
                "profile.tools",
                &file,
                "config_tools_unreadable",
                "all tools",
                json!({"detail": error.to_string()}),
            ));
            return Vec::new();
        }
    };
    let Some(list) = value.get("enabled").and_then(|v| v.as_sequence()) else {
        notices.push(warn(
            "profile.tools",
            &file,
            "config_tools_no_list",
            "all tools",
        ));
        return Vec::new();
    };
    let names = list
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect::<Vec<_>>();
    if names.is_empty() {
        notices.push(warn(
            "profile.tools",
            &file,
            "config_tools_empty",
            "all tools",
        ));
    }
    // `edit` and `splice` locate the same change two ways; a session gets one of them.
    let both = ["edit", "splice"]
        .iter()
        .all(|name| names.iter().any(|listed| listed == name));
    if both {
        notices.push(warn(
            "profile.tools",
            &file,
            "config_tools_edit_splice",
            "edit_dropped",
        ));
        return names.into_iter().filter(|name| name != "edit").collect();
    }
    names
}
