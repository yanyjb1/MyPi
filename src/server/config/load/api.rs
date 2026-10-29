use std::collections::HashSet;
use std::path::PathBuf;

use super::super::model::{self, ResolvedModel};
use super::super::notice::ConfigNotice;
use super::super::paths::{self, ConfigPaths};
use super::super::profile::{self, ResolvedProfile};
use super::super::registry::Value;
use super::super::schema::{
    KEY_CONFIG_SERVER_API_DEFAULT_MODEL, KEY_CONFIG_SERVER_REQUEST_COMPACT_INSTRUCTION_FILE,
};
use super::super::write;
use super::source::child;
use super::types::Config;

impl Config {
    pub fn paths(&self) -> &ConfigPaths {
        &self.inner.paths
    }

    pub fn models(&self) -> impl Iterator<Item = (&str, &model::ModelEntry)> {
        self.inner.models.models()
    }

    pub fn model_by_id(&self, address: &str) -> Option<ResolvedModel> {
        self.inner.models.model_by_id(address)
    }

    pub fn resolve_key(&self, provider: &model::Provider) -> String {
        self.inner.models.resolve_key(provider)
    }

    pub fn default_model(&self, profile: Option<&str>) -> Option<ResolvedModel> {
        let effective = self.effective(profile);
        let configured = effective
            .values
            .get(KEY_CONFIG_SERVER_API_DEFAULT_MODEL)
            .and_then(Value::as_text);
        self.inner.models.default_model(configured)
    }

    pub fn notices(&self, profile: Option<&str>) -> Vec<ConfigNotice> {
        let mut out = self.inner.load_notices.clone();
        out.extend(self.effective(profile).notices.iter().cloned());
        if let Ok(runtime) = self.inner.runtime.lock() {
            out.extend(runtime.iter().cloned());
        }
        let mut seen = HashSet::new();
        out.retain(|notice| {
            seen.insert((
                notice.key.clone(),
                notice.file.clone(),
                notice.problem.clone(),
            ))
        });
        out
    }

    pub fn resolve_profile(&self, name: Option<&str>) -> anyhow::Result<ResolvedProfile> {
        let name = name.unwrap_or(profile::DEFAULT_PROFILE);
        let compact = self
            .effective(Some(name))
            .values
            .get(KEY_CONFIG_SERVER_REQUEST_COMPACT_INSTRUCTION_FILE)
            .and_then(Value::as_text)
            .map(PathBuf::from);
        let mut notices = Vec::new();
        let resolved = profile::resolve(&self.inner.paths, name, compact.as_deref(), &mut notices);
        if let Ok(mut runtime) = self.inner.runtime.lock() {
            runtime.extend(notices);
        }
        resolved
    }

    pub fn set_default_model(&self, id: &str) -> anyhow::Result<PathBuf> {
        self.inner
            .models
            .model_by_id(id)
            .ok_or_else(|| anyhow!("unknown model: {id}"))?;
        let target = self.write_target();
        write::write_key(&target, &["config", "server", "api", "defaultModel"], id)?;
        if let Ok(mut cache) = self.inner.cache.lock() {
            cache.clear();
        }
        Ok(target)
    }

    fn write_target(&self) -> PathBuf {
        let has = |layer: &Option<(PathBuf, Mapping)>| {
            layer
                .as_ref()
                .and_then(|(_, m)| child(m, "server.api"))
                .and_then(|m| m.get("defaultModel"))
                .is_some()
        };
        if has(&self.inner.layers.project)
            && let Some((path, _)) = &self.inner.layers.project
        {
            return path.clone();
        }
        if let Some((path, _)) = &self.inner.layers.global {
            return path.clone();
        }
        self.inner
            .paths
            .global_root()
            .map(|r| r.join(paths::CONFIG_FILE))
            .unwrap_or_else(|| PathBuf::from(paths::CONFIG_FILE))
    }
}
