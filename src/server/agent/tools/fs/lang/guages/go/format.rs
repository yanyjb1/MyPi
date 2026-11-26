// The Go parser: gofmt reads one text from its own input and answers for its syntax alone. It judges a text and never rewrites it.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use super::check::program;
use crate::server::agent::tools::fs::gate;
use crate::server::agent::tools::fs::lang::verdict::{Outcome, Unparsed};
use crate::server::platform::stop::Stop;

// How long one parser round trip may take before its tree is killed.
const LIMIT: Duration = Duration::from_secs(5);

// How many verdicts this process remembers.
const CACHE: usize = 32;

// Whether one text parses. A verdict gofmt reached is remembered; a timeout or a missing binary says nothing about the text and must not outlive the moment.
pub(crate) fn format(text: &str, _path: &Path, stop: &Arc<Stop>) -> Outcome {
    let key = hash(text);
    if let Some(hit) = remembered(&key) {
        return hit;
    }
    let outcome = judge(text, stop);
    if matches!(outcome, Outcome::Parsed | Outcome::Unparsed(_)) {
        remember(key, &outcome);
    }
    outcome
}

// One parser round trip over its own input. What it lays out is thrown away: only its verdict is read.
fn judge(text: &str, stop: &Arc<Stop>) -> Outcome {
    let Some(bin) = program("gofmt") else {
        // No parser to ask is no verdict against the text.
        return Outcome::Parsed;
    };
    let mut cmd = Command::new(&bin);
    cmd.arg("-e");
    match gate::run(cmd, Some(text.as_bytes()), Some(LIMIT), stop) {
        Ok(ran) if ran.stopped => Outcome::Stopped,
        Ok(ran) if ran.timed_out => Outcome::Parsed,
        Ok(ran) if ran.code == Some(0) => Outcome::Parsed,
        Ok(ran) => Outcome::Unparsed(parse(&ran.err)),
        Err(_) => Outcome::Parsed,
    }
}

// gofmt's stderr, cut to the one line and the line number a model can act on. Its first row is the deepest, so it is the one kept.
fn parse(stderr: &str) -> Unparsed {
    for raw in stderr.lines() {
        let trimmed = raw.trim();
        let Some((_, rest)) = trimmed.split_once(':') else {
            continue;
        };
        let Some((line, said)) = rest.split_once(':') else {
            continue;
        };
        let Ok(line) = line.trim().parse::<usize>() else {
            continue;
        };
        let message = rest
            .split_once(": ")
            .map_or(trimmed, |(_, tail)| tail)
            .trim()
            .to_string();
        return Unparsed {
            line: Some(line),
            message: if message.is_empty() {
                format!("gofmt refused the text: {said}")
            } else {
                message
            },
        };
    }
    Unparsed {
        line: None,
        message: "gofmt refused the text".to_string(),
    }
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
