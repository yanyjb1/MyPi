//! Files the model reads back instead of receiving inline: the DOM as HTML, and screenshots.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

/// `<temp>/mypi/web`, one level, the unix timestamp is the whole name.
fn dir() -> PathBuf {
    std::env::temp_dir().join("mypi").join("web")
}

/// Write one file and answer its path.
pub(crate) fn save(extension: &str, bytes: &[u8]) -> Result<PathBuf> {
    let dir = dir();
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    let path = dir.join(format!("{stamp}.{extension}"));
    std::fs::write(&path, bytes).with_context(|| format!("cannot write {}", path.display()))?;
    Ok(path)
}
