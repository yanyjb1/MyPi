// The R parser: air under a deadline. It judges a text and never rewrites it.

use std::path::Path;
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

// Whether one text parses. A verdict air reached is remembered; a timeout or a missing binary says nothing about the text and must not outlive the moment.
pub(crate) fn format(text: &str, path: &Path, stop: &Arc<Stop>) -> Outcome {
    let key = hash(text);
    if let Some(hit) = remembered(&key) {
        return hit;
    }
    let outcome = judge(text, path, stop);
    if matches!(outcome, Outcome::Parsed | Outcome::Unparsed(_)) {
        remember(key, &outcome);
    }
    outcome
}

// One parser round trip, the text handed down its stdin. What air lays out is thrown away: only its verdict is read.
fn judge(text: &str, path: &Path, stop: &Arc<Stop>) -> Outcome {
    let Some(bin) = program("air") else {
        // No parser to ask is no verdict against the text.
        return Outcome::Parsed;
    };
    let mut cmd = Command::new(&bin);
    cmd.args(["format", "--stdin-file-path"]).arg(path);
    match gate::run(cmd, Some(text.as_bytes()), Some(LIMIT), stop) {
        Ok(ran) if ran.stopped => Outcome::Stopped,
        Ok(ran) if ran.timed_out => Outcome::Parsed,
        Ok(ran) if ran.code == Some(0) => Outcome::Parsed,
        Ok(ran) => Outcome::Unparsed(parse(&ran.err)),
        Err(_) => Outcome::Parsed,
    }
}

// air's stderr, cut to the one line and the line number a model can act on. Its first row reads `a.R:2:13 parse ━━━`.
fn parse(stderr: &str) -> Unparsed {
    let mut message: Option<String> = None;
    let mut line = None;
    for raw in stderr.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('━') {
            continue;
        }
        if line.is_none() {
            line = spot(trimmed);
        }
        if message.is_some() {
            continue;
        }
        // The complaint is the one row air opens with a cross and a sentence.
        let said = trimmed.strip_prefix('×').map(str::trim);
        if let Some(said) = said.filter(|said| !said.is_empty() && !said.starts_with("Found")) {
            message = Some(said.trim_end_matches(':').trim().to_string());
        }
    }
    Unparsed {
        line,
        message: message.unwrap_or_else(|| "air refused the text".to_string()),
    }
}

// The line number inside a `a.R:2:13` position, when the sentence carries one.
fn spot(said: &str) -> Option<usize> {
    let bytes: Vec<char> = said.chars().collect();
    for at in 0..bytes.len() {
        if bytes[at] != ':' {
            continue;
        }
        let digits: String = bytes[at + 1..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if digits.is_empty() || bytes.get(at + 1 + digits.len()) != Some(&':') {
            continue;
        }
        return digits.parse().ok();
    }
    None
}

// A program of the toolchain: beside this executable first, then whatever PATH holds.
pub(super) fn program(name: &str) -> Option<std::path::PathBuf> {
    static FOUND: LazyLock<Mutex<std::collections::HashMap<String, Option<std::path::PathBuf>>>> =
        LazyLock::new(|| Mutex::new(std::collections::HashMap::new()));
    if let Some(hit) = lock(&FOUND).get(name) {
        return hit.clone();
    }
    let beside = gate::beside(name);
    let found = beside.is_file().then_some(beside).or_else(|| on_path(name));
    lock(&FOUND).insert(name.to_string(), found.clone());
    found
}

// The one a shell would find first, walking the search path's own directories.
fn on_path(name: &str) -> Option<std::path::PathBuf> {
    // Windows keeps the extension in the name it runs, so the search asks for that name.
    let named = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let dirs = std::env::var_os("PATH")?;
    std::env::split_paths(&dirs)
        .map(|dir| dir.join(&named))
        .find(|candidate| candidate.is_file())
}

// The verdicts this process has already reached, oldest first.
static MEMO: LazyLock<Mutex<Vec<(u64, Outcome)>>> = LazyLock::new(|| Mutex::new(Vec::new()));

fn remembered(key: &u64) -> Option<Outcome> {
    lock(&MEMO)
        .iter()
        .find(|(held, _)| held == key)
        .map(|(_, outcome)| outcome.clone())
}

fn remember(key: u64, outcome: &Outcome) {
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
