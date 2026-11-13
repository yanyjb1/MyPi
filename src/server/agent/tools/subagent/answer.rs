//! The answer file: the one name a sub-session's final text lands under, and its one write.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::server::agent::tools::workspace::write_atomic;

/// The first free name: the plain stem, then `-1`, `-2` and on. Names the same call has already
/// handed out are taken too, so two children of one call never share a file.
pub(crate) fn free_avoiding(dir: &Path, stem: &str, taken: &BTreeSet<String>) -> Result<String> {
    let stem = clean(stem)?;
    if !taken.contains(&stem) && !file(dir, &stem).exists() {
        return Ok(stem);
    }
    for n in 1..=999u32 {
        let candidate = format!("{stem}-{n}");
        if !taken.contains(&candidate) && !file(dir, &candidate).exists() {
            return Ok(candidate);
        }
    }
    anyhow::bail!("`{stem}` already has a thousand answers")
}

/// The file one name lands in.
pub(crate) fn file(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.md"))
}

/// Write the answer, stamped: a draft that carries an older stamp loses it.
pub(crate) fn write(path: &Path, text: &str) -> Result<()> {
    write_atomic(
        path,
        &format!("{}\n\n{}", generated(), strip_generated(text)),
    )
}

/// One plain stem: no separators, no hidden file.
fn clean(stem: &str) -> Result<String> {
    let stem = stem.trim();
    anyhow::ensure!(!stem.is_empty(), "the answer name is empty");
    anyhow::ensure!(
        !stem.contains('/') && !stem.contains('\\'),
        "the answer name must not contain a path separator"
    );
    anyhow::ensure!(
        !stem.starts_with('.'),
        "the answer name must not start with a dot"
    );
    Ok(stem.to_string())
}

fn generated() -> String {
    let now = crate::server::platform::time::unix_now();
    let (y, m, d, h, min, s) = crate::server::platform::time::local_parts(now);
    let offset = crate::server::platform::time::offset_at(now);
    format!(
        "<!-- generated: {y:04}-{m:02}-{d:02} {h:02}:{min:02}:{s:02} (UTC{}) -->",
        offset_label(offset)
    )
}

fn offset_label(offset: i32) -> String {
    let sign = if offset < 0 { '-' } else { '+' };
    let seconds = offset.unsigned_abs();
    let hours = seconds / 3600;
    let minutes = (seconds % 3600) / 60;
    if minutes == 0 {
        format!("{sign}{hours}")
    } else {
        format!("{sign}{hours}:{minutes:02}")
    }
}

fn strip_generated(text: &str) -> &str {
    if text.starts_with("<!-- generated:") {
        match text.split_once('\n') {
            Some((_, rest)) => rest.trim_start_matches('\n'),
            None => "",
        }
    } else {
        text
    }
}
