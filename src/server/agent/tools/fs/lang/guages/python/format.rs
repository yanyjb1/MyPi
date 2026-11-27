// The Python parser: ruff under a deadline. It judges a text and never rewrites it.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use super::check::program;
use crate::platform::stop::Stop;
use crate::server::agent::tools::fs::gate;
use crate::server::agent::tools::fs::lang::verdict::{Outcome, Unparsed};

// How long one parser round trip may take before its tree is killed.
const LIMIT: Duration = Duration::from_secs(5);

// How many verdicts this process remembers.
const CACHE: usize = 32;

// Whether one text parses. A verdict ruff reached is remembered; a timeout or a missing binary says nothing about the text and must not outlive the moment.
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

// One parser round trip. What it lays out is thrown away: only its verdict is read.
fn judge(text: &str, path: &Path, stop: &Arc<Stop>) -> Outcome {
    let Some(bin) = program("ruff") else {
        // No parser to ask is no verdict against the text.
        return Outcome::Parsed;
    };
    let mut cmd = Command::new(&bin);
    cmd.args(["format", "--stdin-filename"]).arg(path).arg("-");
    match gate::run(cmd, Some(text.as_bytes()), Some(LIMIT), stop) {
        Ok(ran) if ran.stopped => Outcome::Stopped,
        Ok(ran) if ran.timed_out => Outcome::Parsed,
        Ok(ran) if ran.code == Some(0) => Outcome::Parsed,
        Ok(ran) => Outcome::Unparsed(parse(&ran.err)),
        Err(_) => Outcome::Parsed,
    }
}

// ruff's stderr, cut to the position a model can act on.
fn parse(stderr: &str) -> Unparsed {
    let mut message: Option<String> = None;
    let mut place = None;
    for raw in stderr.lines() {
        let trimmed = raw.trim();
        let trimmed = trimmed
            .strip_prefix("[stderr]")
            .map_or(trimmed, str::trim_start);
        if place.is_none() {
            place = spot(trimmed);
        }
        if message.is_some() {
            continue;
        }
        let said = trimmed.strip_prefix("error: ").or_else(|| {
            trimmed
                .strip_prefix("error[")
                .and_then(|rest| rest.split_once("]: ").map(|(_, tail)| tail))
        });
        if let Some(said) = said {
            message = Some(said.trim_end_matches(':').trim().to_string());
        }
    }
    Unparsed {
        line: place.map(|(line, _)| line),
        column: place.and_then(|(_, column)| column),
        message: message.unwrap_or_else(|| "ruff refused the text".to_string()),
    }
}

// The line and the column inside a `bad.py:1:7:` position, when the sentence carries them.
fn spot(said: &str) -> Option<(usize, Option<usize>)> {
    let bytes: Vec<char> = said.chars().collect();
    for at in 0..bytes.len() {
        if bytes[at] != ':' {
            continue;
        }
        let digits: String = bytes[at + 1..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        let after = at + 1 + digits.len();
        if digits.is_empty() || bytes.get(after) != Some(&':') {
            continue;
        }
        let Ok(line) = digits.parse::<usize>() else {
            continue;
        };
        let column: String = bytes[after + 1..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        return Some((line, column.parse().ok()));
    }
    None
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
