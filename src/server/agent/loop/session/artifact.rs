//! The session's artifact directory. It dies with the live state.

use std::path::{Path, PathBuf};

/// One session's temporary artifact directory.
pub struct ArtifactStore {
    dir: PathBuf,
}

impl ArtifactStore {
    pub fn new(session: i64) -> Self {
        let dir = std::env::temp_dir().join(format!("mypi-{session}"));
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for ArtifactStore {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
