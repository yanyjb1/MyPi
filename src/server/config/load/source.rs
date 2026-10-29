use std::path::Path;

use anyhow::Context as _;
use serde_yaml::{Mapping, Value as Yaml};

use super::super::notice::{ConfigNotice, warn};
use super::super::paths::{self, ConfigPaths};
use super::super::profile;
use super::super::registry;
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
                Err(problem) => notices.push(warn("config", &file, &problem, "next layer")),
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
                "unknown top-level key",
                "ignored",
            ));
        }
    }
}

pub(super) fn bootstrap(paths: &ConfigPaths, default_model: Option<&str>) -> anyhow::Result<()> {
    let Some(root) = paths.global_root() else {
        return Ok(());
    };
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
    let has_default = paths.profiles_dirs().iter().any(|(_, dir)| {
        dir.join(profile::DEFAULT_PROFILE)
            .join(paths::SYSTEM_FILE)
            .is_file()
    });
    if has_default {
        return Ok(());
    }
    let profile = root
        .join(paths::PROFILES_SUBDIR)
        .join(profile::DEFAULT_PROFILE);
    std::fs::create_dir_all(&profile)?;
    for (name, body) in [
        (paths::SYSTEM_FILE, profile::omp_system()),
        (paths::COMPACT_FILE, profile::compact_instruction()),
    ] {
        let path = profile.join(name);
        if !path.exists() {
            std::fs::write(&path, body)
                .with_context(|| format!("failed to write {}", path.display()))?;
            println!("mypi: wrote {}", path.display());
        }
    }
    Ok(())
}

pub(super) fn child<'a>(map: &'a Mapping, path: &str) -> Option<&'a Mapping> {
    let mut cur = map;
    for seg in path.split('.').filter(|s| !s.is_empty()) {
        cur = cur.get(seg)?.as_mapping()?;
    }
    Some(cur)
}
