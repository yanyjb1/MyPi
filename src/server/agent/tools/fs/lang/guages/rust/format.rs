// The Rust parser: rustfmt under a deadline, with the edition the nearest manifest names. It judges a text and never rewrites it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use crate::server::agent::tools::fs::gate;
use crate::server::agent::tools::fs::lang::verdict::{Outcome, Unparsed};
use crate::server::platform::stop::Stop;

// How long one parser round trip may take before its tree is killed.
const LIMIT: Duration = Duration::from_secs(5);

// How many verdicts this process remembers.
const CACHE: usize = 32;

// The edition an older manifest still accepts.
const DEFAULT_EDITION: &str = "2021";

// Whether one text parses. A verdict rustfmt reached is remembered; a timeout or a missing binary says nothing about the text and must not outlive the moment.
pub(crate) fn format(text: &str, path: &Path, stop: &Arc<Stop>) -> Outcome {
    let edition = edition(path);
    let key = (edition.clone(), hash(text));
    if let Some(hit) = remembered(&key) {
        return hit;
    }
    let outcome = judge(text, &edition, stop);
    if matches!(outcome, Outcome::Parsed | Outcome::Unparsed(_)) {
        remember(key, &outcome);
    }
    outcome
}

// The crate's edition, from the nearest `Cargo.toml` above the file. Read once per directory.
pub(super) fn edition(path: &Path) -> String {
    static EDITIONS: LazyLock<Mutex<HashMap<PathBuf, String>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    let Some(dir) = path.parent() else {
        return DEFAULT_EDITION.to_string();
    };
    if let Some(held) = lock(&EDITIONS).get(dir) {
        return held.clone();
    }
    let found = edition_at(dir);
    lock(&EDITIONS).insert(dir.to_path_buf(), found.clone());
    found
}

// The edition of the crate rooted at `dir`, one entry per ancestor until a manifest appears.
fn edition_at(dir: &Path) -> String {
    for at in dir.ancestors() {
        let manifest = at.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        return std::fs::read_to_string(&manifest)
            .ok()
            .and_then(|text| edition_of(&text))
            .unwrap_or_else(|| DEFAULT_EDITION.to_string());
    }
    DEFAULT_EDITION.to_string()
}

// The first `edition = "…"` in a manifest.
fn edition_of(manifest: &str) -> Option<String> {
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some(rest) = line.strip_prefix("edition") else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if matches!(value, "2015" | "2018" | "2021" | "2024") {
            return Some(value.to_string());
        }
    }
    None
}

// One parser round trip. What it lays out is thrown away: only its verdict is read.
fn judge(text: &str, edition: &str, stop: &Arc<Stop>) -> Outcome {
    let bin = gate::beside("rustfmt");
    let mut cmd = Command::new(&bin);
    cmd.args(["--edition", edition, "--emit", "stdout", "--quiet"]);
    match gate::run(cmd, Some(text.as_bytes()), Some(LIMIT), stop) {
        Ok(ran) if ran.stopped => Outcome::Stopped,
        // A parser that never answered is no judge: nothing is known, so nothing is refused.
        Ok(ran) if ran.timed_out => Outcome::Parsed,
        Ok(ran) if ran.code == Some(0) => Outcome::Parsed,
        Ok(ran) => Outcome::Unparsed(parse(&ran.err)),
        Err(_) => Outcome::Parsed,
    }
}

// rustfmt's stderr, cut to the one line and the line number a model can act on.
fn parse(stderr: &str) -> Unparsed {
    let mut message: Option<String> = None;
    let mut line = None;
    for raw in stderr.lines() {
        let trimmed = raw.trim();
        if let Some(rest) = trimmed.strip_prefix("--> ") {
            line = rest.split(':').nth(1).and_then(|spot| spot.parse().ok());
            continue;
        }
        if message.is_some() {
            continue;
        }
        let cleaned = trimmed.strip_prefix("error: ").or_else(|| {
            trimmed
                .strip_prefix("error[")
                .and_then(|rest| rest.split_once("]: ").map(|(_, tail)| tail))
        });
        if let Some(cleaned) = cleaned {
            message = Some(cleaned.trim_end_matches(':').to_string());
        }
    }
    Unparsed {
        line,
        message: message.unwrap_or_else(|| "rustfmt refused the text".to_string()),
    }
}

type Key = (String, u64);

// The verdicts this process has already reached, oldest first.
static MEMO: LazyLock<Mutex<Vec<(Key, Outcome)>>> = LazyLock::new(|| Mutex::new(Vec::new()));

fn remembered(key: &Key) -> Option<Outcome> {
    lock(&MEMO)
        .iter()
        .find(|(held, _)| held == key)
        .map(|(_, outcome)| outcome.clone())
}

fn remember(key: Key, outcome: &Outcome) {
    let mut held = lock(&MEMO);
    while held.len() >= CACHE {
        held.remove(0);
    }
    held.push((key, outcome.clone()));
}

// A lock that survives a panicking holder: the state behind it is a cache, never a promise.
fn lock<T>(held: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    held.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// A text's content hash.
fn hash(text: &str) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}
