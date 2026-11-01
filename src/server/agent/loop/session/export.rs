//! Full texts live in the database; this writes one out as a file when a call asks for it.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

use crate::server::agent::tools::contract::Export;
use crate::server::storage::Reader;

/// One process's export folder, plus the database to read the text from.
pub struct Files {
    db: PathBuf,
    dir: PathBuf,
}

impl Files {
    pub fn new(db: PathBuf, dir: PathBuf) -> Self {
        Self { db, dir }
    }
}

impl Export for Files {
    fn dir(&self) -> &Path {
        &self.dir
    }

    fn path(&self, reference: &str) -> Result<PathBuf> {
        let target = self.dir.join(reference);
        if target.is_file() {
            return Ok(target);
        }
        let reader = Reader::open(&self.db)?;
        let rows = reader.artifacts_by_ref(reference)?;
        let row = rows
            .first()
            .ok_or_else(|| anyhow!("unknown artifact #{reference}"))?;
        let content = row["content"]
            .as_str()
            .ok_or_else(|| anyhow!("artifact #{reference} carries no text"))?;
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("cannot create {}", self.dir.display()))?;
        let tmp = target.with_extension(format!("tmp-{}", std::process::id()));
        std::fs::write(&tmp, content).with_context(|| format!("cannot write {}", tmp.display()))?;
        std::fs::rename(&tmp, &target)
            .with_context(|| format!("cannot place {}", target.display()))?;
        Ok(target)
    }
}
