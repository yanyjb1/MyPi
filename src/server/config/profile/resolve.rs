use std::path::Path;

use anyhow::{Context as _, bail};

use super::super::notice::{ConfigNotice, warn};
use super::super::paths::{self, ConfigPaths};
use super::discover::collect;
use super::types::{DEFAULT_PROFILE, OMP_PROFILE, ResolvedProfile};

pub fn omp_system() -> &'static str {
    super::super::prompts::omp_system()
}

pub fn compact_instruction() -> &'static str {
    super::super::prompts::compact_instruction()
}

fn builtin(name: &str) -> bool {
    name == DEFAULT_PROFILE || name == OMP_PROFILE
}

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
            Err(error) if builtin(name) => {
                notices.push(warn(
                    &format!("profile.{name}"),
                    &dir.display().to_string(),
                    &error.to_string(),
                    "compiled copy",
                ));
            }
            Err(error) => return Err(error),
        }
    }
    if builtin(name) {
        return Ok(ResolvedProfile {
            system: omp_system().to_string(),
            tools: None,
            compact_instruction: instruction(None, compact_file, notices),
        });
    }
    let mut names: Vec<String> = found.keys().cloned().collect();
    for builtin in [DEFAULT_PROFILE, OMP_PROFILE] {
        if !names.iter().any(|n| n.as_str() == builtin) {
            names.push(builtin.to_string());
        }
    }
    names.sort();
    bail!("unknown profile: {name}. Available: {}", names.join(", "));
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
        compact_instruction: instruction(compact.as_deref(), compact_file, notices),
    })
}

fn instruction(
    profile: Option<&Path>,
    configured: Option<&Path>,
    notices: &mut Vec<ConfigNotice>,
) -> String {
    if let Some(path) = profile {
        match std::fs::read_to_string(path) {
            Ok(text) => return text,
            Err(e) => notices.push(warn(
                "profile.compactInstruction",
                &path.display().to_string(),
                &e.to_string(),
                "compiled copy",
            )),
        }
    }
    if let Some(path) = configured {
        match std::fs::read_to_string(path) {
            Ok(text) => return text,
            Err(e) => notices.push(warn(
                "config.server.request.compact.instructionFile",
                &path.display().to_string(),
                &e.to_string(),
                "compiled copy",
            )),
        }
    }
    compact_instruction().to_string()
}

fn load_tools(path: &Path, notices: &mut Vec<ConfigNotice>) -> Vec<String> {
    let file = path.display().to_string();
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            notices.push(warn(
                "profile.tools",
                &file,
                &error.to_string(),
                "all tools",
            ));
            return Vec::new();
        }
    };
    let value: serde_yaml::Value = match serde_yaml::from_str(&text) {
        Ok(value) => value,
        Err(error) => {
            notices.push(warn(
                "profile.tools",
                &file,
                &error.to_string(),
                "all tools",
            ));
            return Vec::new();
        }
    };
    let Some(list) = value.get("enabled").and_then(|v| v.as_sequence()) else {
        notices.push(warn(
            "profile.tools",
            &file,
            "tools.yaml needs an `enabled:` list",
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
            "empty enabled list",
            "all tools",
        ));
    }
    names
}
