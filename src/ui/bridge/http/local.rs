// What the door answers itself: this machine's directories, and the name a cited page gives itself.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::ipc::contract::Frame;

// How many names one listing carries; a longer directory offers no more.
const MAX: usize = 2000;

// `local/list`: every name one directory holds, directories marked, and the directory that was read.
pub fn list(payload: &Value) -> Value {
    let word = payload
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let base = payload
        .get("cwd")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let head = match word.rfind('/') {
        Some(at) => &word[..=at],
        None => "",
    };
    let dir = std::fs::canonicalize(resolve(head, base)).unwrap_or_else(|_| resolve(head, base));
    let mut found: Vec<(bool, String)> = match std::fs::read_dir(&dir) {
        Ok(read) => read
            .flatten()
            .take(MAX)
            .map(|one| {
                let dir = one.file_type().map(|kind| kind.is_dir()).unwrap_or(false);
                (dir, one.file_name().to_string_lossy().to_string())
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    found.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase()))
    });
    let entries: Vec<Value> = found
        .into_iter()
        .map(|(dir, name)| json!({"name": name, "dir": dir}))
        .collect();
    json!({"dir": dir.to_string_lossy(), "entries": entries})
}

// One local frame, whichever of the two it is: the door's own look at this machine, or at the pages it cites.
pub fn answer(frame: &Frame) -> Option<Value> {
    match frame.name.as_str() {
        "list" => Some(list(&frame.payload)),
        "title" => Some(super::title::lookup(&frame.payload)),
        _ => None,
    }
}

// The directory the word's own head names: relative paths stand on the session's own directory.
fn resolve(head: &str, base: &str) -> PathBuf {
    if head.is_empty() {
        return if base.is_empty() {
            PathBuf::from(".")
        } else {
            PathBuf::from(base)
        };
    }
    if let Some(rest) = head.strip_prefix('~')
        && let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))
    {
        return Path::new(&home).join(rest.trim_start_matches(['/', '\\']));
    }
    let named = Path::new(head);
    if named.is_absolute() || base.is_empty() {
        return named.to_path_buf();
    }
    Path::new(base).join(named)
}
