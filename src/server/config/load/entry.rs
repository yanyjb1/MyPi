use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, anyhow};

use super::super::model;
use super::super::paths;
use super::super::paths::ConfigPaths;
use super::source;
use super::types::{Config, Inner, Layers};

impl Config {
    pub fn load() -> anyhow::Result<Self> {
        Self::load_from(ConfigPaths::from_env())
    }

    pub fn load_from(paths: ConfigPaths) -> anyhow::Result<Self> {
        Self::read(paths, true)
    }

    fn read(paths: ConfigPaths, bootstrap_files: bool) -> anyhow::Result<Self> {
        let models_path = paths.models_file().ok_or_else(|| {
            anyhow!(
                "no {} under the global root {} — create it there; the program never writes it",
                paths::MODELS_FILE,
                paths
                    .global_root()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(none)".into())
            )
        })?;
        let text = std::fs::read_to_string(&models_path)
            .with_context(|| format!("failed to read {}", models_path.display()))?;
        let (models, mut notices) = model::parse(&text, &models_path.display().to_string())?;
        let first = models
            .alphabetical_default()
            .map(|m| format!("{}:{}", m.provider_name, m.entry.id));
        if bootstrap_files {
            source::bootstrap(&paths, first.as_deref())?;
        }
        let layers = Layers::read(&paths, &mut notices);
        Ok(Config {
            inner: Arc::new(Inner {
                paths,
                models,
                layers,
                cache: Mutex::new(HashMap::new()),
                load_notices: notices,
                runtime: Mutex::new(Vec::new()),
            }),
        })
    }

    pub fn reload(&self) -> anyhow::Result<Config> {
        Self::read(self.inner.paths.clone(), false)
    }
}
