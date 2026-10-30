//! The state that outlives one turn.

use std::path::PathBuf;

use super::artifact::ArtifactStore;
use super::book::Birth;
use super::settings::Settings;
use super::transport::Transport;

/// One session's live state.
pub struct Live {
    pub provider: String,
    pub model: String,
    pub system: String,
    pub tools: Vec<serde_json::Value>,
    pub cwd: PathBuf,
    pub settings: Settings,
    pub artifacts: ArtifactStore,
}

impl Live {
    pub fn new(birth: &Birth, settings: &Settings) -> Self {
        let cwd = birth
            .cwd
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        Self {
            provider: birth.provider.clone(),
            model: birth.model.clone(),
            system: birth.system.clone(),
            tools: birth.tools.clone(),
            cwd,
            settings: settings.clone(),
            artifacts: ArtifactStore::new(birth.session),
        }
    }

    pub fn transport(&self) -> Transport {
        Transport::new(self.provider.clone(), self.model.clone())
    }
}
