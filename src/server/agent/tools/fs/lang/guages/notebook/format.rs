// The notebook parser: ruff under a deadline, and what the notebook's own JSON says. It judges a text and never rewrites it.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use super::check::program;
use super::scan;
use crate::server::agent::tools::fs::gate;
use crate::server::agent::tools::fs::lang::verdict::{Outcome, Unparsed};
use crate::server::platform::stop::Stop;

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
        Ok(ran) => Outcome::Unparsed(parse(&ran.err, path, text)),
        Err(_) => Outcome::Parsed,
    }
}

// ruff's stderr, cut to what a model can act on. A notebook's position reads `name:cell:row:column`, so the cell's own rows turn that row into a file line, the sentence names the cell it fell in, and the position itself is cut away.
fn parse(stderr: &str, path: &Path, text: &str) -> Unparsed {
    let cells = scan::cells(text);
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut message: Option<String> = None;
    let mut line = None;
    let mut named = None;
    for raw in stderr.lines() {
        let trimmed = raw.trim();
        let trimmed = trimmed
            .strip_prefix("[stderr]")
            .map_or(trimmed, str::trim_start);
        if line.is_none()
            && let Some((word, at, rest)) = position(trimmed, &name, &cells)
        {
            line = Some(at);
            named = Some(word);
            if message.is_none() {
                message = Some(rest.trim_end_matches(':').trim().to_string());
            }
            continue;
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
    let message = message.unwrap_or_else(|| "ruff refused the text".to_string());
    Unparsed {
        line,
        message: match named {
            Some(word) => format!("{word}: {message}"),
            None => message,
        },
    }
}

// The cell a `name:cell:row:column` position names, the file line that row stands on, and what the sentence says after the position.
fn position<'a>(
    said: &'a str,
    name: &str,
    cells: &[scan::Cell],
) -> Option<(String, usize, &'a str)> {
    let after = said.split_once(name)?.1;
    let (cell, row, rest) = numbers(after)?;
    let found = cells.iter().find(|one| one.index == cell)?;
    let word = format!("cell {} ({})", found.index, found.kind);
    Some((word, scan::place(found, row), rest))
}

// The cell and row a position names, and what follows its column.
fn numbers(after: &str) -> Option<(usize, usize, &str)> {
    let (cell, rest) = after.strip_prefix(':')?.split_once(':')?;
    let (row, rest) = rest.split_once(':')?;
    let (_column, rest) = rest.split_once(':')?;
    Some((cell.parse().ok()?, row.parse().ok()?, rest))
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
