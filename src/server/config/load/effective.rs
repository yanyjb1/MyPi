use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_yaml::Mapping;

use super::super::model;
use super::super::notice::warn;
use super::super::paths;
use super::super::profile;
use super::super::registry::{self, Value};
use super::check::{check, walk_unknown};
use super::source::{child, read_layer, unknown_top};
use super::types::{Config, Effective};

impl Config {
    pub fn effective(&self, profile: Option<&str>) -> Arc<Effective> {
        let cache_key = profile.unwrap_or("").to_string();
        if let Ok(cache) = self.inner.cache.lock()
            && let Some(hit) = cache.get(&cache_key)
        {
            return hit.clone();
        }

        let mut notices = Vec::new();
        let mut overlay: Option<(PathBuf, Mapping)> = None;
        if let Some(name) = profile {
            let dir = profile::locate(&self.inner.paths, name, &mut notices);
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
        if let Some((path, map)) = &self.inner.layers.project {
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
        let effective = Arc::new(Effective { values, notices });
        if let Ok(mut cache) = self.inner.cache.lock() {
            cache.insert(cache_key, effective.clone());
        }
        effective
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
