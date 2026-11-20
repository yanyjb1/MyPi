use std::path::Path;

use anyhow::Context as _;
use serde_json::json;
use serde_yaml::{Mapping, Value as Yaml};

use super::super::notice::{ConfigNotice, warn, warn_args};
use super::super::paths::{self, ConfigPaths};
use super::super::profile;
use super::super::registry;
use super::super::rules;
use super::super::write;
use super::types::Layers;

impl Layers {
    pub(super) fn read(paths: &ConfigPaths, notices: &mut Vec<ConfigNotice>) -> Self {
        let mut project = None;
        let mut global = None;
        for (origin, path) in paths.config_files() {
            let file = path.display().to_string();
            match read_layer(&path) {
                Ok(map) => {
                    unknown_top(&path, notices);
                    match origin {
                        paths::Origin::Project => project = Some((path.clone(), map)),
                        paths::Origin::Global => global = Some((path.clone(), map)),
                    }
                }
                Err(problem) => notices.push(warn_args(
                    "config",
                    &file,
                    "config_unreadable",
                    "next_layer",
                    json!({"detail": problem}),
                )),
            }
        }
        Layers { project, global }
    }
}

pub(super) fn read_layer(path: &Path) -> Result<Mapping, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let root: Yaml = serde_yaml::from_str(&text).map_err(|e| e.to_string())?;
    let top = root
        .as_mapping()
        .ok_or_else(|| "the file must be a mapping".to_string())?;
    let config = top
        .get("config")
        .ok_or_else(|| "missing the `config:` block".to_string())?;
    config
        .as_mapping()
        .cloned()
        .ok_or_else(|| "`config:` must be a mapping".to_string())
}

pub(super) fn unknown_top(path: &Path, notices: &mut Vec<ConfigNotice>) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    let Ok(root) = serde_yaml::from_str::<Yaml>(&text) else {
        return;
    };
    let Some(map) = root.as_mapping() else { return };
    for key in map.keys() {
        if let Some(name) = key.as_str()
            && name != "config"
        {
            notices.push(warn(
                name,
                &path.display().to_string(),
                "config_unknown_top_key",
                "ignored",
            ));
        }
    }
}

pub(super) fn bootstrap(
    paths: &ConfigPaths,
    default_model: Option<&str>,
    notices: &mut Vec<ConfigNotice>,
) -> anyhow::Result<()> {
    let Some(root) = paths.global_root() else {
        return Ok(());
    };
    rules::install(root)?;
    let config = root.join(paths::CONFIG_FILE);
    if paths.config_files().is_empty() {
        let text = registry::template();
        std::fs::create_dir_all(root)?;
        std::fs::write(&config, &text)
            .with_context(|| format!("failed to write {}", config.display()))?;
        if let Some(id) = default_model {
            write::write_key(&config, &["config", "server", "api", "defaultModel"], id)?;
        }
        println!("mypi: wrote {}", config.display());
    }
    for builtin in profile::BUILTINS {
        seed(paths, root, builtin, notices);
    }
    Ok(())
}

/// One built-in profile, written only where no root already carries it: a directory of that name is
/// the user's, and is never touched. A file that cannot be written leaves the compiled copy in
/// force, so a read-only root costs a notice and nothing else.
fn seed(
    paths: &ConfigPaths,
    root: &Path,
    builtin: &profile::Builtin,
    notices: &mut Vec<ConfigNotice>,
) {
    let held = paths
        .profiles_dirs()
        .iter()
        .any(|(_, dir)| dir.join(builtin.name).join(paths::SYSTEM_FILE).is_file());
    if held {
        return;
    }
    let dir = root.join(paths::PROFILES_SUBDIR).join(builtin.name);
    if let Err(error) = std::fs::create_dir_all(&dir) {
        notices.push(warn_args(
            &format!("profile.{}", builtin.name),
            &dir.display().to_string(),
            "config_profile_unwritable",
            "builtin",
            json!({"detail": error.to_string()}),
        ));
        return;
    }
    for (name, body) in [
        (paths::SYSTEM_FILE, (builtin.system)()),
        (paths::COMPACT_FILE, (builtin.compact)()),
    ] {
        let path = dir.join(name);
        if path.exists() {
            continue;
        }
        match std::fs::write(&path, body) {
            Ok(()) => println!("mypi: wrote {}", path.display()),
            Err(error) => notices.push(warn_args(
                &format!("profile.{}", builtin.name),
                &path.display().to_string(),
                "config_profile_unwritable",
                "builtin",
                json!({"detail": error.to_string()}),
            )),
        }
    }
}

pub(super) fn child<'a>(map: &'a Mapping, path: &str) -> Option<&'a Mapping> {
    let mut cur = map;
    for seg in path.split('.').filter(|s| !s.is_empty()) {
        cur = cur.get(seg)?.as_mapping()?;
    }
    Some(cur)
}
