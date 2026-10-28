use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;

use crate::server::agent::tools;
use crate::server::storage::decode::{ARTIFACT_COLS, artifact_from_row};
use crate::server::storage::{Reader, base36};

/// Artifact fetcher backed by one database file.
pub(crate) struct ArtifactStore {
    db: PathBuf,
    made: Mutex<Vec<PathBuf>>,
}

impl ArtifactStore {
    pub(crate) fn new(db: PathBuf) -> Self {
        Self {
            db,
            made: Mutex::new(Vec::new()),
        }
    }

    fn dir() -> PathBuf {
        crate::server::platform::env::temp_dir().join("mypi-arts")
    }
}

impl tools::Artifacts for ArtifactStore {
    fn path_for(&self, id: i64) -> Result<PathBuf> {
        let dir = Self::dir();
        let path = dir.join(base36(id));
        if path.exists() {
            return Ok(path);
        }
        let conn = Reader::open(&self.db)?;
        let row = conn
            .query_row(
                &format!("SELECT {ARTIFACT_COLS} FROM artifacts WHERE block_id = ?1"),
                [id],
                artifact_from_row,
            )
            .optional()?;
        Reader::close(conn);
        let artifact = row.with_context(|| format!("no artifact is bound to block {id}"))?;
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("cannot make the artifact directory {}", dir.display()))?;
        write_temp(&path, &artifact.content)?;
        self.made
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(path.clone());
        Ok(path)
    }
}

impl Drop for ArtifactStore {
    fn drop(&mut self) {
        let made = std::mem::take(&mut *self.made.lock().unwrap_or_else(|e| e.into_inner()));
        for path in made {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn write_temp(path: &Path, content: &str) -> Result<()> {
    std::fs::write(path, content)
        .with_context(|| format!("cannot write the artifact {}", path.display()))?;
    Ok(())
}
