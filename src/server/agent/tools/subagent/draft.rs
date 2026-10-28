//! The in-memory draft a child edits, and the one file it leaves behind.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result};

use crate::server::agent::tools::bash::edit;
use crate::server::agent::tools::workspace::write_atomic;

#[derive(Default)]
struct Draft {
    text: String,
    submitted: bool,
}

/// One map for every child in this process.
#[derive(Clone, Default)]
pub(crate) struct Drafts {
    inner: Arc<Mutex<HashMap<i64, Draft>>>,
}

impl Drafts {
    pub(crate) fn write(&self, child: i64, text: String) {
        self.with(child, |draft| draft.text = text);
    }

    pub(crate) fn edit(&self, child: i64, old: &str, new: &str) -> Result<String> {
        self.with(child, |draft| {
            let (text, _, message) = edit::exact_text(&draft.text, old, new)?;
            draft.text = text;
            Ok(message)
        })
    }

    pub(crate) fn mass_edit(&self, child: i64, edits: &[(usize, String)]) -> Result<String> {
        self.with(child, |draft| {
            let (text, _, message) = edit::mass_text(&draft.text, edits)?;
            draft.text = text;
            Ok(message)
        })
    }

    pub(crate) fn read(&self, child: i64) -> String {
        self.with(child, |draft| {
            if draft.text.is_empty() {
                "(empty draft)".to_string()
            } else {
                draft.text.clone()
            }
        })
    }

    pub(crate) fn submit(&self, root: &Path, child: i64) -> Result<PathBuf> {
        let text = self.with(child, |draft| {
            anyhow::ensure!(!draft.text.trim().is_empty(), "the draft is empty");
            Ok(draft.text.clone())
        })?;
        let path = write_file(root, child, &text)?;
        self.with(child, |draft| draft.submitted = true);
        Ok(path)
    }

    pub(crate) fn finish(&self, root: &Path, child: i64, fallback: Option<&str>) -> Result<bool> {
        let (submitted, text) = self.with(child, |draft| {
            let text = if !draft.text.trim().is_empty() {
                Some(draft.text.clone())
            } else {
                fallback.map(str::to_string)
            };
            (draft.submitted, text)
        });
        if submitted {
            return Ok(true);
        }
        let Some(text) = text else {
            return Ok(false);
        };
        if text.trim().is_empty() {
            return Ok(false);
        }
        write_file(root, child, &text)?;
        self.with(child, |draft| draft.submitted = true);
        Ok(true)
    }

    fn with<R>(&self, child: i64, f: impl FnOnce(&mut Draft) -> R) -> R {
        let mut map = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        f(map.entry(child).or_default())
    }
}

/// The only place a final file's path is decided.
pub(crate) fn path(root: &Path, child: i64) -> PathBuf {
    root.join(".mypi")
        .join("answers")
        .join(format!("{child}.md"))
}

fn write_file(root: &Path, child: i64, text: &str) -> Result<PathBuf> {
    let path = path(root, child);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot make the answer directory {}", parent.display()))?;
    }
    let body = strip_generated(text);
    let content = format!("{}\n\n{body}", generated());
    write_atomic(&path, &content)?;
    Ok(path)
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
