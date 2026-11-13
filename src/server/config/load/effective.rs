use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_yaml::Mapping;

use super::super::model;
use super::super::notice::warn;
use super::super::paths;
use super::super::paths::ConfigPaths;
use super::super::profile;
use super::super::registry::{self, Value};
use super::check::{check, walk_unknown};
use super::source::{child, read_layer, unknown_top};
use super::types::{Config, Effective, Layers, ProjectScope};

impl Config {
    pub fn effective(&self, profile: Option<&str>) -> Arc<Effective> {
        let cache_key = profile.unwrap_or("").to_string();
        if let Ok(cache) = self.inner.cache.lock()
            && let Some(hit) = cache.get(&cache_key)
        {
            return hit.clone();
        }
        let effective = self.compose(
            &self.inner.paths,
            self.inner.layers.project.as_ref(),
            profile,
        );
        if let Ok(mut cache) = self.inner.cache.lock() {
            cache.insert(cache_key, effective.clone());
        }
        effective
    }

    /// One directory's values: its project layer above the global one, under the profile's overlay.
    pub fn effective_in(&self, cwd: &Path, profile: Option<&str>) -> Arc<Effective> {
        let scope = self.scope(cwd);
        let cache_key = format!("{}\u{1}{}", cwd.display(), profile.unwrap_or(""));
        if let Ok(cache) = self.inner.scoped.lock()
            && let Some(hit) = cache.get(&cache_key)
        {
            return hit.clone();
        }
        let effective = self.compose(&scope.paths, scope.layer.as_ref(), profile);
        if let Ok(mut cache) = self.inner.scoped.lock() {
            cache.insert(cache_key, effective.clone());
        }
        effective
    }

    /// One directory's roots and project layer, read once and kept for the process.
    pub(super) fn scope(&self, cwd: &Path) -> Arc<ProjectScope> {
        let root = paths::project_dir(cwd);
        if let Ok(scopes) = self.inner.projects.lock()
            && let Some(hit) = scopes.get(&root)
        {
            return hit.clone();
        }
        let paths = ConfigPaths::for_cwd(cwd);
        let mut notices = Vec::new();
        let layer = Layers::read(&paths, &mut notices).project;
        if let Ok(mut runtime) = self.inner.runtime.lock() {
            runtime.extend(notices);
        }
        let scope = Arc::new(ProjectScope { paths, layer });
        if let Ok(mut scopes) = self.inner.projects.lock() {
            scopes.insert(root, scope.clone());
        }
        scope
    }

    /// The merge itself: one value per key, from the highest layer that has a valid one.
    fn compose(
        &self,
        paths: &ConfigPaths,
        project: Option<&(PathBuf, Mapping)>,
        profile: Option<&str>,
    ) -> Arc<Effective> {
        let mut notices = Vec::new();
        let mut overlay: Option<(PathBuf, Mapping)> = None;
        if let Some(name) = profile {
            let dir = profile::locate(paths, name, &mut notices);
            if let Some(dir) = dir {
                let path = dir.join(paths::PROFILE_CONFIG);
                if path.is_file() {
                    match read_layer(&path) {
                        Ok(map) => {
                            unknown_top(&path, &mut notices);
                            overlay = Some((path, map));
                        }
                        Err(problem) => notices.push(warn(
                            &format!("profile.{name}"),
                            &path.display().to_string(),
                            &problem,
                            "next layer",
                        )),
                    }
                }
            }
        }

        let mut layers: Vec<(String, &Mapping)> = Vec::new();
        if let Some((path, map)) = &overlay {
            layers.push((path.display().to_string(), map));
        }
        if let Some((path, map)) = project {
            layers.push((path.display().to_string(), map));
        }
        if let Some((path, map)) = &self.inner.layers.global {
            layers.push((path.display().to_string(), map));
        }

        let mut values: HashMap<&'static str, Value> = HashMap::new();
        for key in registry::config_keys() {
            let mut chosen: Option<(usize, Value)> = None;
            let mut bad: Option<(String, String)> = None;
            for (index, (file, map)) in layers.iter().enumerate() {
                let Some(raw) = child(map, key.path).and_then(|m| m.get(key.name)) else {
                    continue;
                };
                match key.parse(raw) {
                    Ok(value) => {
                        chosen = Some((index, value));
                        break;
                    }
                    Err(problem) => {
                        if bad.is_none() {
                            bad = Some((file.clone(), problem));
                        }
                    }
                }
            }
            let used = match &chosen {
                Some((index, _)) => layers[*index].0.clone(),
                None => "default".to_string(),
            };
            if let Some((file, problem)) = bad {
                notices.push(warn(key.full, &file, &problem, &used));
            }
            let value = chosen
                .map(|(_, value)| value)
                .unwrap_or_else(|| key.default_value());
            values.insert(key.full, value);
        }

        let mut warned: HashSet<String> = HashSet::new();
        for (file, map) in &layers {
            walk_unknown(map, "config", file, &mut notices, &mut warned);
        }

        check(&mut values, &mut notices, &self.inner.models);
        Arc::new(Effective { values, notices })
    }

    pub fn get(&self, key: &str, profile: Option<&str>) -> Result<Value, String> {
        if model::is_model_key(key) {
            return self.inner.models.get(key);
        }
        if registry::lookup(key).is_none() {
            return Err(format!("unknown key: {key}"));
        }
        self.effective(profile)
            .values
            .get(key)
            .cloned()
            .ok_or_else(|| format!("unknown key: {key}"))
    }

    pub fn get_json(&self, key: &str, profile: Option<&str>) -> Result<serde_json::Value, String> {
        self.get(key, profile).map(|value| value.to_json())
    }
}
