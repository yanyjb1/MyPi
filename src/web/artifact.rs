//! Files the model reads back instead of receiving inline: the DOM as HTML, and screenshots.

use std::path::PathBuf;

use anyhow::{Context, Result, anyhow};

/// `<temp>/mypi/web`, one level, named for the site the file came from.
fn dir() -> PathBuf {
    std::env::temp_dir().join("mypi").join("web")
}

/// Write one file and answer its path. A site's files are numbered, so two writes at the same
/// moment do not land on each other.
pub(crate) fn save(site: &str, extension: &str, bytes: &[u8]) -> Result<PathBuf> {
    let dir = dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let free = (1..)
        .map(|n| dir.join(format!("{site}-{n}.{extension}")))
        .find(|path| !path.exists());
    let path = free.ok_or_else(|| anyhow!("no free name left in {}", dir.display()))?;
    std::fs::write(&path, bytes).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(path)
}
