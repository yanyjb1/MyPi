use std::collections::HashSet;
use std::path::PathBuf;

use super::super::model::{self, ResolvedModel};
use super::super::notice::ConfigNotice;
use super::super::paths::{self, ConfigPaths};
use super::super::profile::{self, ResolvedProfile};
use super::super::registry::Value;
use super::super::schema::{
    KEY_CONFIG_SERVER_AGENT_SUBAGENT_MAX_TOOL_CALLS, KEY_CONFIG_SERVER_AGENT_TOOLS_BASH_TIMEOUT,
    KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_BIN, KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_EXTRA_ARGS,
    KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_HEADLESS, KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_PORT,
    KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_PROFILE_DIR,
    KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_USER_AGENT,
    KEY_CONFIG_SERVER_AGENT_TOOLS_OUTPUT_MAX_BYTES, KEY_CONFIG_SERVER_AGENT_TOOLS_OUTPUT_MAX_LINES,
    KEY_CONFIG_SERVER_API_DEFAULT_MODEL, KEY_CONFIG_SERVER_REQUEST_COMPACT_INSTRUCTION_FILE,
};
use super::super::write;
use super::source::child;
use super::types::Config;

impl Config {
    pub fn paths(&self) -> &ConfigPaths {
        &self.inner.paths
    }

    /// The inline budget one tool result gets: lines, bytes.
    pub fn limits(&self) -> (usize, usize) {
        let effective = self.effective(None);
        let int = |key: &'static str| {
            effective
                .values
                .get(key)
                .and_then(Value::as_int)
                .unwrap_or(0)
                .max(0) as usize
        };
        (
            int(KEY_CONFIG_SERVER_AGENT_TOOLS_OUTPUT_MAX_LINES),
            int(KEY_CONFIG_SERVER_AGENT_TOOLS_OUTPUT_MAX_BYTES),
        )
    }

    /// One bash command's wall-clock cap.
    pub fn bash_timeout(&self) -> std::time::Duration {
        let seconds = self
            .effective(None)
            .values
            .get(KEY_CONFIG_SERVER_AGENT_TOOLS_BASH_TIMEOUT)
            .and_then(Value::as_int)
            .unwrap_or(600)
            .max(1);
        std::time::Duration::from_secs(seconds as u64)
    }

    /// What the web tool launches with.
    pub fn browser(&self) -> crate::server::config::BrowserOptions {
        let effective = self.effective(None);
        let value = |key: &'static str| effective.values.get(key);
        let path = |key: &'static str| value(key).and_then(Value::as_text).map(PathBuf::from);
        crate::server::config::BrowserOptions {
            bin: path(KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_BIN),
            port: value(KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_PORT)
                .and_then(Value::as_int)
                .and_then(|port| u16::try_from(port).ok()),
            profile_dir: path(KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_PROFILE_DIR),
            headless: value(KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_HEADLESS)
                .and_then(Value::as_bool)
                .unwrap_or(true),
            user_agent: value(KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_USER_AGENT)
                .and_then(Value::as_text)
                .map(str::to_string),
            extra_args: value(KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_EXTRA_ARGS)
                .and_then(Value::as_list)
                .unwrap_or(&[])
                .to_vec(),
        }
    }

    /// The tool-call budget a sub-session gets when its call names none.
    pub fn subagent_budget(&self) -> u32 {
        self.effective(None)
            .values
            .get(KEY_CONFIG_SERVER_AGENT_SUBAGENT_MAX_TOOL_CALLS)
            .and_then(Value::as_int)
            .unwrap_or(1000)
            .max(0)
            .min(u32::MAX as i64) as u32
    }

    /// Where exported full texts land.
    pub fn export_dir(&self) -> PathBuf {
        crate::server::platform::env::downloads_dir()
            .unwrap_or_else(crate::server::platform::env::temp_dir)
    }

    /// The project root and its two folders.
    pub fn project_paths(&self) -> super::super::paths::ProjectDirs {
        paths::project_dirs()
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
