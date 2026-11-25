// The Python format: ruff format under a deadline, with the wrapper comments that walk its laid-out output back to the runs a call wrote.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use super::check::program;
use crate::server::agent::tools::fs::gate;
use crate::server::agent::tools::fs::lang::verdict::{Draft, Outcome, Place, Unparsed};
use crate::server::platform::stop::Stop;

// How long one formatter round trip may take before its tree is killed.
const LIMIT: Duration = Duration::from_secs(5);

// How many bytes of formatted text this process remembers.
const CACHE: usize = 32 * 1024;

// The word a run's wrapper comment starts with.
const MARK: &str = "mypi:splice";

// Format one text, or say what stopped it. Only a verdict ruff reached is remembered: a timeout or a missing binary says nothing about the text and must not outlive the moment.
pub(crate) fn format(draft: &Draft, path: &Path, stop: &Arc<Stop>) -> Outcome {
    let tag = tag(draft.text);
    let input = wrapped(draft, &tag);
    let key = hash(&input);
    if let Some(hit) = remembered(&key) {
        return hit;
    }
    let outcome = formatter(&input, &tag, draft.runs.len(), path, stop);
    if matches!(outcome, Outcome::Formatted { .. } | Outcome::Unparsed(_)) {
        remember(key, &outcome);
    }
    outcome
}

// The text handed to ruff: every run wrapped in its own pair of marker comments, each inheriting the indentation of the line it stands beside, so Python's blocks stay whole.
fn wrapped(draft: &Draft, tag: &str) -> String {
    if draft.runs.is_empty() {
        return draft.text.to_string();
    }
    let raw: Vec<&str> = draft.text.lines().collect();
    let mut lines: Vec<String> = raw.iter().map(|line| (*line).to_string()).collect();
    // Bottom up, so the runs above keep their own numbers while they are still being used.
    for (at, (first, last)) in draft.runs.iter().enumerate().rev() {
        let open = first.saturating_sub(1);
        let close = (*last).max(first.saturating_sub(1));
        let closing = format!("{}# {tag} {at} close", lead(&raw, close));
        let opening = format!("{}# {tag} {at} open", lead(&raw, open));
        lines.insert(close.min(raw.len()), closing);
        lines.insert(open.min(raw.len()), opening);
    }
    let mut text = lines.join("\n");
    if draft.text.ends_with('\n') && !text.is_empty() {
        text.push('\n');
    }
    text
}

// The blank runs in front of one line: a marker comment takes them, so it may not break the block it stands in.
fn lead(raw: &[&str], at: usize) -> String {
    let Some(line) = raw.get(at) else {
        return String::new();
    };
    line.chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

// One formatter round trip, and the walk back to the runs it was given.
fn formatter(input: &str, tag: &str, runs: usize, path: &Path, stop: &Arc<Stop>) -> Outcome {
    let Some(bin) = program("ruff") else {
        return Outcome::Undecided(
            "failed to find `ruff`; put it beside this executable or on PATH".to_string(),
        );
    };
    let mut cmd = Command::new(&bin);
    cmd.args(["format", "--stdin-filename"]).arg(path).arg("-");
    match gate::run(cmd, Some(input.as_bytes()), Some(LIMIT), stop) {
        Ok(ran) if ran.stopped => Outcome::Stopped,
        Ok(ran) if ran.timed_out => Outcome::Undecided(format!(
            "ruff did not finish in {} s, so it was killed",
            LIMIT.as_secs()
        )),
        Ok(ran) if ran.code == Some(0) && runs == 0 => Outcome::Formatted {
            text: ran.out,
            runs: Vec::new(),
        },
        Ok(ran) if ran.code == Some(0) => Outcome::Formatted {
            text: unmarked(&ran.out, tag),
            runs: marks(&ran.out, tag),
        },
        Ok(ran) => Outcome::Unparsed(parse(&ran.err)),
        Err(e) => Outcome::Undecided(format!(
            "failed to run `{}`: {e}; put ruff beside this executable or on PATH",
            bin.display()
        )),
    }
}

// This run's marker word: the constant, and a fingerprint of the text it marks.
fn tag(base: &str) -> String {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    base.hash(&mut hasher);
    format!("{}:{:016x}", MARK, hasher.finish())
}

// One edge of a run's wrapper comment.
enum Edge {
    Open,
    Close,
}

// The run index and edge this line carries, when it is one of the wrapper comments.
fn mark_at(line: &str, tag: &str) -> Option<(usize, Edge)> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix('#')?.trim_start();
    let mut words = rest.split_whitespace();
    if words.next()? != tag {
        return None;
    }
    let index = words.next()?.parse().ok()?;
    let edge = match words.next()? {
        "open" => Edge::Open,
        "close" => Edge::Close,
        _ => return None,
    };
    Some((index, edge))
}

// Where each run's own lines ended up, in the order the runs were handed over. A marker comment takes no line number of its own, so its neighbours are the exact lines the run became.
fn marks(text: &str, tag: &str) -> Vec<Place> {
    let mut blocks: Vec<Place> = Vec::new();
    let mut open: Option<(usize, usize)> = None;
    let mut at = 0usize;
    for line in text.lines() {
        let Some((index, edge)) = mark_at(line, tag) else {
            at += 1;
            continue;
        };
        match edge {
            Edge::Open => open = Some((index, at)),
            Edge::Close => {
                if let Some((index, first)) = open.take() {
                    if blocks.len() <= index {
                        blocks.resize(index + 1, None);
                    }
                    blocks[index] = Some((first + 1, at));
                }
            }
        }
    }
    blocks
}

// The formatted text without its marker lines.
fn unmarked(text: &str, tag: &str) -> String {
    let kept: Vec<&str> = text
        .lines()
        .filter(|line| mark_at(line, tag).is_none())
        .collect();
    let mut out = kept.join("\n");
    if text.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    out
}

// ruff's stderr, cut to the one line and the line number a model can act on.
fn parse(stderr: &str) -> Unparsed {
    let mut message: Option<String> = None;
    let mut line = None;
    for raw in stderr.lines() {
        let trimmed = raw.trim();
        let trimmed = trimmed
            .strip_prefix("[stderr]")
            .map_or(trimmed, str::trim_start);
        if line.is_none() {
            line = spot(trimmed);
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
        line,
        message: message.unwrap_or_else(|| "ruff refused the text".to_string()),
    }
}

// The line number inside a `bad.py:1:7:` position, when the sentence carries one.
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

// The formatted texts this process has already produced, oldest first.
static MEMO: LazyLock<Mutex<Vec<(u64, Outcome)>>> = LazyLock::new(|| Mutex::new(Vec::new()));

fn remembered(key: &u64) -> Option<Outcome> {
    lock(&MEMO)
        .iter()
        .find(|(held, _)| held == key)
        .map(|(_, outcome)| outcome.clone())
}

fn remember(key: u64, outcome: &Outcome) {
    // A text too large to cache is simply not cached: the retries that follow pay one process.
    let size = weigh(outcome);
    if size > CACHE {
        return;
    }
    let mut held = lock(&MEMO);
    while held.iter().map(|(_, held)| weigh(held)).sum::<usize>() + size > CACHE {
        held.remove(0);
    }
    held.push((key, outcome.clone()));
}

// How many bytes one cached outcome holds.
fn weigh(outcome: &Outcome) -> usize {
    match outcome {
        Outcome::Formatted { text, .. } => text.len(),
        Outcome::Unparsed(_) | Outcome::Undecided(_) | Outcome::Stopped => 0,
    }
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
