//! The state that outlives one turn.

use std::path::PathBuf;
use std::sync::Arc;

use super::Transport;
use super::artifact::ArtifactStore;
use super::book::Birth;

/// One session's live state.
pub struct Live {
    pub provider: String,
    pub model: String,
    pub system: String,
    pub tools: Vec<serde_json::Value>,
    /// The compaction instruction template; it comes from the profile the caller resolved.
    pub compact: String,
    pub cwd: PathBuf,
    pub artifacts: Arc<ArtifactStore>,
}

impl Live {
    pub fn new(birth: &Birth) -> Self {
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
            compact: birth.compact.clone(),
            cwd,
            artifacts: Arc::new(ArtifactStore::new(birth.session)),
        }
    }

    pub fn transport(&self) -> Transport {
        Transport::new(self.provider.clone(), self.model.clone())
    }
}
