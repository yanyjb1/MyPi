//! The session's artifact files. They die with the live state.

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

    pub fn path_for(&self, reference: &str) -> PathBuf {
        self.dir.join(reference)
    }

    pub fn materialize(&self, reference: &str, content: &str) -> std::io::Result<PathBuf> {
        let path = self.path_for(reference);
        std::fs::write(&path, content)?;
        Ok(path)
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
