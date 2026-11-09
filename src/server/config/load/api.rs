use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use anyhow::anyhow;
use serde_yaml::Mapping;

use super::super::model::{self, ResolvedModel};
use super::super::notice::ConfigNotice;
use super::super::paths::{self, ConfigPaths};
use super::super::profile::{self, ResolvedProfile};
use super::super::registry::{self, Value};
use super::super::schema::{
    KEY_CONFIG_SERVER_AGENT_LOOP_MAX_TOOL_CALLS, KEY_CONFIG_SERVER_AGENT_LOOP_REQUEST_MAX_RETRIES,
    KEY_CONFIG_SERVER_AGENT_LOOP_REQUEST_RETRY_INTERVAL,
    KEY_CONFIG_SERVER_AGENT_LOOP_REQUEST_RETRY_TIMEOUT, KEY_CONFIG_SERVER_AGENT_LOOP_SCROOGE,
    KEY_CONFIG_SERVER_AGENT_SUBAGENT_MAX_TOOL_CALLS, KEY_CONFIG_SERVER_AGENT_TOOLS_ALIASES,
    KEY_CONFIG_SERVER_AGENT_TOOLS_BASH_TIMEOUT, KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_BIN,
    KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_EXTRA_ARGS,
    KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_IDLE_SECS, KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_PORT,
    KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_PROFILE_DIR,
    KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_USER_AGENT,
    KEY_CONFIG_SERVER_AGENT_TOOLS_OUTPUT_MAX_BYTES, KEY_CONFIG_SERVER_AGENT_TOOLS_OUTPUT_MAX_LINES,
    KEY_CONFIG_SERVER_API_DEFAULT_MODEL, KEY_CONFIG_SERVER_REQUEST_COMPACT_INSTRUCTION_FILE,
    KEY_CONFIG_SERVER_REQUEST_COMPACT_RETAIN_TAIL, KEY_CONFIG_SERVER_REQUEST_STREAMING,
    KEY_CONFIG_WEB_BRING_TO_FRONT,
};
use super::super::write;
use super::aliases;
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

    /// The user's aliases from config, fixed for every session.
    pub fn aliases(&self) -> BTreeMap<String, String> {
        self.effective(None)
            .values
            .get(KEY_CONFIG_SERVER_AGENT_TOOLS_ALIASES)
            .and_then(Value::as_list)
            .unwrap_or(&[])
            .iter()
            .filter_map(|entry| aliases::split(entry).ok())
            .collect()
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
            idle: std::time::Duration::from_secs(
                value(KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_IDLE_SECS)
                    .and_then(Value::as_int)
                    .unwrap_or(600)
                    .max(1) as u64,
            ),
            user_agent: value(KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_USER_AGENT)
                .and_then(Value::as_text)
                .map(str::to_string),
            extra_args: value(KEY_CONFIG_SERVER_AGENT_TOOLS_BROWSER_EXTRA_ARGS)
                .and_then(Value::as_list)
                .unwrap_or(&[])
                .to_vec(),
        }
    }

    /// Whether a verification challenge should raise the browser window.
    pub fn bring_to_front(&self) -> bool {
        self.effective(None)
            .values
            .get(KEY_CONFIG_WEB_BRING_TO_FRONT)
            .and_then(Value::as_bool)
            .unwrap_or(true)
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

    /// Every provider the request half can reach, credentials expanded.
    pub fn providers(&self) -> Vec<crate::server::agent::request::Provider> {
        self.inner
            .models
            .providers
            .iter()
            .filter_map(|provider| {
                let protocol =
                    crate::server::agent::request::ProtocolId::parse(&provider.api).ok()?;
                Some(crate::server::agent::request::Provider {
                    name: provider.name.clone(),
                    base_url: provider.base_url.clone(),
                    protocol,
                    api_key: self.inner.models.resolve_key(provider),
                    auth_header: provider.auth_header,
                })
            })
            .collect()
    }

    /// The declared output cap of every configured model, keyed `provider:model`.
    pub fn output_caps(&self) -> std::collections::BTreeMap<String, u32> {
        let mut caps = std::collections::BTreeMap::new();
        for (provider, entry) in self.models() {
            let address = format!("{provider}:{}", entry.id);
            let Some(cap) = entry
                .max_output_tokens
                .and_then(|cap| u32::try_from(cap).ok())
            else {
                panic!("models.yml: {address} has no maxOutputTokens")
            };
            caps.insert(address, cap);
        }
        caps
    }

    /// Tool-round fence; `None` turns it off.
    pub fn tool_rounds(&self) -> Option<usize> {
        match self
            .effective(None)
            .values
            .get(KEY_CONFIG_SERVER_AGENT_LOOP_MAX_TOOL_CALLS)
        {
            Some(Value::BoolOrInt(registry::BoolOrInt::Int(n))) => {
                Some(usize::try_from(*n).unwrap_or(0))
            }
            Some(Value::Int(n)) => Some(usize::try_from(*n).unwrap_or(0)),
            _ => None,
        }
    }

    /// Forbid a model change mid-session.
    pub fn scrooge(&self) -> bool {
        self.effective(None)
            .values
            .get(KEY_CONFIG_SERVER_AGENT_LOOP_SCROOGE)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }

    /// Retries after the first attempt.
    pub fn retry_attempts(&self) -> u32 {
        self.effective(None)
            .values
            .get(KEY_CONFIG_SERVER_AGENT_LOOP_REQUEST_MAX_RETRIES)
            .and_then(Value::as_int)
            .unwrap_or(10)
            .max(0) as u32
    }

    /// How long to wait between attempts.
    pub fn retry_interval(&self) -> std::time::Duration {
        let seconds = self
            .effective(None)
            .values
            .get(KEY_CONFIG_SERVER_AGENT_LOOP_REQUEST_RETRY_INTERVAL)
            .and_then(Value::as_int)
            .unwrap_or(30)
            .max(0) as u64;
        std::time::Duration::from_secs(seconds)
    }

    /// How long one attempt waits for the response head.
    pub fn retry_timeout(&self) -> std::time::Duration {
        let seconds = self
            .effective(None)
            .values
            .get(KEY_CONFIG_SERVER_AGENT_LOOP_REQUEST_RETRY_TIMEOUT)
            .and_then(Value::as_int)
            .unwrap_or(120)
            .max(1) as u64;
        std::time::Duration::from_secs(seconds)
    }

    /// One piece per chunk rather than per block.
    pub fn streaming_immediate(&self) -> bool {
        self.effective(None)
            .values
            .get(KEY_CONFIG_SERVER_REQUEST_STREAMING)
            .and_then(Value::as_text)
            .map(|text| text == "immediate")
            .unwrap_or(true)
    }

    /// Characters a compaction keeps when the caller names no budget.
    pub fn retain_tail(&self) -> usize {
        self.effective(None)
            .values
            .get(KEY_CONFIG_SERVER_REQUEST_COMPACT_RETAIN_TAIL)
            .and_then(Value::as_int)
            .unwrap_or(20000)
            .max(0) as usize
    }

    /// Every profile name a session may name.
    pub fn profiles(&self) -> Vec<String> {
        profile::list(&self.inner.paths, &mut Vec::new())
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
