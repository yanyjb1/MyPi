// The web parser: one biome round trip on a text, under a deadline, and the verdict read off it. It judges a text and never rewrites it.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use super::kind;
use crate::platform::stop::Stop;
use crate::server::agent::tools::fs::gate;
use crate::server::agent::tools::fs::lang::verdict::{Outcome, Unparsed};

// How long one biome round trip may take before its tree is killed.
const LIMIT: Duration = Duration::from_secs(10);

// How many verdicts this process remembers.
const CACHE: usize = 32;

// Whether one text parses. A verdict biome reached is remembered; a missing binary is no verdict against the text.
pub(crate) fn format(text: &str, path: &Path, stop: &Arc<Stop>) -> Outcome {
    // The verdict belongs to a kind as much as to a text: the same bytes are a page, a stylesheet or a script by the name they are handed over under.
    let key = hash(&format!("{}{text}", kind(path).unwrap_or("")));
    if let Some(hit) = remembered(&key) {
        return hit;
    }
    let outcome = judge(text, path, stop);
    if matches!(outcome, Outcome::Parsed | Outcome::Unparsed(_)) {
        remember(key, &outcome);
    }
    outcome
}

// One biome round trip on the text: the answer it gave, or `None` when the binary never answered. `switch` starts the formatter of this kind; a commit asks for none, and takes the repository's own answer instead.
pub(super) fn round(
    text: &str,
    path: &Path,
    switch: Option<&str>,
    stop: &Arc<Stop>,
) -> Option<gate::Ran> {
    let mut cmd = Command::new(gate::beside("biome"));
    cmd.arg("format")
        .arg(format!("--stdin-file-path={}", path.display()));
    if let Some(switch) = switch {
        cmd.arg(switch);
    }
    gate::run(cmd, Some(text.as_bytes()), Some(LIMIT), stop).ok()
}

// The switch that turns biome's formatter on for this kind. A repository may leave its formatters off, and a formatter that is off answers nothing about the text: it hands the text back as it came, whatever the text holds.
fn flag(path: &Path) -> &'static str {
    match kind(path) {
        Some("css") => "--css-formatter-enabled=true",
        Some("js") => "--javascript-formatter-enabled=true",
        _ => "--html-formatter-enabled=true",
    }
}

// One parser round trip. What it lays out is thrown away: only its verdict is read.
fn judge(text: &str, path: &Path, stop: &Arc<Stop>) -> Outcome {
    let Some(ran) = round(text, path, Some(flag(path)), stop) else {
        return Outcome::Parsed;
    };
    if ran.stopped {
        return Outcome::Stopped;
    }
    if ran.timed_out || ran.code == Some(0) {
        return Outcome::Parsed;
    }
    Outcome::Unparsed(parse(&ran.err))
}

// biome's stderr, cut to the position a model can act on.
fn parse(stderr: &str) -> Unparsed {
    let mut message: Option<String> = None;
    let mut place = None;
    for raw in stderr.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if place.is_none() {
            place = spot(trimmed);
        }
        if message.is_some() {
            continue;
        }
        let said = trimmed.strip_prefix('×').map(str::trim).or_else(|| {
            trimmed
                .strip_prefix("× ")
                .map(str::trim)
                .filter(|said| !said.starts_with('['))
        });
        if let Some(said) = said
            && !said.starts_with("The content was not formatted")
        {
            message = Some(said.trim_end_matches(':').trim().to_string());
        }
    }
    Unparsed {
        line: place.map(|(line, _)| line),
        column: place.and_then(|(_, column)| column),
        message: message.unwrap_or_else(|| "biome refused the text".to_string()),
    }
}

// The line and the column inside a `stdin:1:7` position, when the sentence carries them.
fn spot(said: &str) -> Option<(usize, Option<usize>)> {
    let chars: Vec<char> = said.chars().collect();
    for at in 0..chars.len() {
        if chars[at] != ':' {
            continue;
        }
        let digits: String = chars[at + 1..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        let after = at + 1 + digits.len();
        if digits.is_empty() || chars.get(after) != Some(&':') {
            continue;
        }
        let Ok(line) = digits.parse::<usize>() else {
            continue;
        };
        let column: String = chars[after + 1..]
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
