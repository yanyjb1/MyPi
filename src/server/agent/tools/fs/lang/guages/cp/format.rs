// The CP parser: clang's own front end run as a syntax judge under a deadline. It judges a text and never rewrites it.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use super::check::program;
use crate::server::agent::tools::fs::gate;
use crate::server::agent::tools::fs::lang::verdict::{Outcome, Unparsed};
use crate::server::platform::stop::Stop;

// How long one parser round trip may take.
const LIMIT: Duration = Duration::from_secs(20);

// How many verdicts this process remembers.
const CACHE: usize = 32;

// The standard a lone text is read under, when nothing in the project says better.
const STANDARD: &str = "-std=c++20";

// Whether one text parses. A verdict clang reached is remembered; a timeout, a missing binary and a header the text was never given say nothing about the text and must not outlive the moment. A verdict judges this text in this place, so the place is part of what is kept.
pub(crate) fn format(text: &str, path: &Path, stop: &Arc<Stop>) -> Outcome {
    let half = language(path);
    let key = (half.to_string(), path.to_path_buf(), hash(text));
    if let Some(hit) = remembered(&key) {
        return hit;
    }
    let outcome = judge(text, path, half, stop);
    if matches!(outcome, Outcome::Parsed | Outcome::Unparsed(_)) {
        remember(key, &outcome);
    }
    outcome
}

// Which half of the language this path is written in: the one suffix C keeps for itself.
fn language(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "c" => "c",
        _ => "c++",
    }
}

// One parser round trip over stdin, so the text the model wrote and nothing on disk is what gets judged.
fn judge(text: &str, path: &Path, half: &str, stop: &Arc<Stop>) -> Outcome {
    let Some(bin) = program("clang") else {
        // No parser to ask is no verdict against the text.
        return Outcome::Parsed;
    };
    let mut cmd = Command::new(&bin);
    cmd.args(["-fsyntax-only", "-x", half, "-ferror-limit=1"]);
    if half == "c++" {
        cmd.arg(STANDARD);
    }
    // The project's own headers come from its compile database; stdin has no directory of its own, so a quoted include beside the file is searched for from that directory.
    let mut dirs = super::facts::include_dirs(path);
    if let Some(dir) = path.parent() {
        dirs.push(dir.to_path_buf());
    }
    for dir in dirs {
        cmd.arg("-I").arg(dir);
    }
    cmd.arg("-");
    match gate::run(cmd, Some(text.as_bytes()), Some(LIMIT), stop) {
        Ok(ran) if ran.stopped => Outcome::Stopped,
        Ok(ran) if ran.timed_out => Outcome::Parsed,
        Ok(ran) if ran.code == Some(0) => Outcome::Parsed,
        Ok(ran) => match first(&ran.err) {
            // A text that cannot reach a header was never given a fair reading: the parser says nothing about the text itself.
            Some((_, _, message)) if environmental(&message) => Outcome::Parsed,
            Some((line, column, message)) => Outcome::Unparsed(Unparsed {
                line,
                column,
                message,
            }),
            None => Outcome::Parsed,
        },
        Err(_) => Outcome::Parsed,
    }
}

// Whether a diagnostic speaks of the world around the text rather than of the text: a header that is not there, a flag that names nothing.
fn environmental(message: &str) -> bool {
    message.contains("file not found")
        || message.contains("unsupported option")
        || message.contains("unknown argument")
}

// The first diagnostic clang reported, cut to its line, its column and its sentence.
fn first(err: &str) -> Option<(Option<usize>, Option<usize>, String)> {
    for raw in err.lines() {
        // Every diagnostic starts with the file, then the line: `<stdin>:` for a text handed over on a pipe.
        let raw = raw.strip_prefix("<stdin>:").unwrap_or(raw);
        let Some((place, rest)) = raw.split_once(':') else {
            continue;
        };
        let Ok(line) = place.parse::<usize>() else {
            continue;
        };
        let (head, tail) = rest.split_once(':').unwrap_or((rest, ""));
        let column = head.trim().parse::<usize>().ok();
        let rest = if tail.is_empty() { rest } else { tail };
        let said = rest.trim_start();
        let message = said
            .strip_prefix("fatal error: ")
            .or_else(|| said.strip_prefix("error: "));
        if let Some(message) = message {
            return Some((Some(line), column, message.trim().to_string()));
        }
    }
    None
}

// What one verdict was reached on: the half, the place, and the text.
type Key = (String, PathBuf, u64);

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
