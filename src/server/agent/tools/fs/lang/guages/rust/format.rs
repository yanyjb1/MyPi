// The Rust format: rustfmt under a deadline, with the edition the nearest manifest names, and the wrapper comments that walk its laid-out output back to the runs a call wrote.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use crate::server::agent::tools::fs::gate;
use crate::server::agent::tools::fs::lang::verdict::{Draft, Outcome, Place, Unparsed};
use crate::server::platform::stop::Stop;

// How long one formatter round trip may take before its tree is killed.
const LIMIT: Duration = Duration::from_secs(5);

// How many bytes of formatted text this process remembers.
const CACHE: usize = 32 * 1024;

// The edition an older manifest still accepts.
const DEFAULT_EDITION: &str = "2021";

// The word a run's wrapper comment starts with. The comment carries a fingerprint of the text, so a line of a file that reads like a marker cannot be mistaken for one.
const MARK: &str = "mypi:splice";

// Format one text, or say what stopped it. Only a verdict rustfmt reached is remembered: a timeout or a missing binary says nothing about the text and must not outlive the moment.
pub(crate) fn format(draft: &Draft, path: &Path, stop: &Arc<Stop>) -> Outcome {
    let edition = edition(path);
    let tag = tag(draft.text);
    let input = wrapped(draft, &tag);
    let key = (edition.clone(), hash(&input));
    if let Some(hit) = remembered(&key) {
        return hit;
    }
    let outcome = formatter(&input, &tag, draft.runs.len(), &edition, stop);
    if matches!(outcome, Outcome::Formatted { .. } | Outcome::Unparsed(_)) {
        remember(key, &outcome);
    }
    outcome
}

// The crate's edition, from the nearest `Cargo.toml` above the file. Read once per directory.
fn edition(path: &Path) -> String {
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

// The text handed to rustfmt: every run wrapped in its own pair of marker comments, so the laid-out output can be walked back to it. A text no run names goes as it is.
fn wrapped(draft: &Draft, tag: &str) -> String {
    if draft.runs.is_empty() {
        return draft.text.to_string();
    }
    let mut lines: Vec<String> = draft.text.lines().map(str::to_string).collect();
    // Bottom up, so the runs above keep their own numbers while they are still being used.
    for (at, (first, last)) in draft.runs.iter().enumerate().rev() {
        let open = first.saturating_sub(1);
        let close = (*last).max(first.saturating_sub(1));
        lines.insert(close, format!("// {tag} {at} close"));
        lines.insert(open, format!("// {tag} {at} open"));
    }
    let mut text = lines.join("\n");
    if draft.text.ends_with('\n') && !text.is_empty() {
        text.push('\n');
    }
    text
}

// One formatter round trip, and the walk back to the runs it was given.
fn formatter(input: &str, tag: &str, runs: usize, edition: &str, stop: &Arc<Stop>) -> Outcome {
    let bin = rustfmt();
    let mut cmd = Command::new(&bin);
    cmd.args(["--edition", edition, "--emit", "stdout", "--quiet"]);
    match gate::run(cmd, Some(input.as_bytes()), Some(LIMIT), stop) {
        Ok(ran) if ran.stopped => Outcome::Stopped,
        Ok(ran) if ran.timed_out => Outcome::Undecided(format!(
            "rustfmt did not finish in {} s, so it was killed",
            LIMIT.as_secs()
        )),
        Ok(ran) if ran.code == Some(0) && runs == 0 => Outcome::Formatted {
            text: ran.out,
            runs: Vec::new(),
        },
        Ok(ran) if ran.code == Some(0) => {
            let found = marks(&ran.out, tag);
            let (text, dropped) = unmarked(&ran.out, tag, &found);
            let runs = found
                .into_iter()
                .map(|place| place.map(|(first, last)| placed(first, last, &dropped)))
                .collect();
            Outcome::Formatted { text, runs }
        }
        Ok(ran) => Outcome::Unparsed(parse(&ran.err)),
        Err(e) => Outcome::Undecided(format!(
            "failed to run `{}`: {e}; put rustfmt beside this executable or on PATH",
            bin.display()
        )),
    }
}

// The `rustfmt` to run: the one beside this executable first, then whatever PATH holds.
fn rustfmt() -> PathBuf {
    static FOUND: LazyLock<PathBuf> = LazyLock::new(|| gate::beside("rustfmt"));
    FOUND.clone()
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

// One edge of a run's wrapper comment.
enum Edge {
    Open,
    Close,
}

// This run's marker word: the constant, and a fingerprint of the text it marks.
fn tag(base: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    use std::hash::{Hash as _, Hasher as _};
    base.hash(&mut hasher);
    format!("{}:{:016x}", MARK, hasher.finish())
}

// The run index and edge this line carries, when it is one of the wrapper comments.
fn mark_at(line: &str, tag: &str) -> Option<(usize, Edge)> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("//")?.trim_start();
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

// Where each run's own lines ended up, in the order the runs were handed over. A marker comment takes no line number of its own, so its neighbours are the exact lines the run became. Positions count the text without the marker lines; `unmarked` reads them in the text that is written.
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

// The formatted text without its marker lines, and how many lines before each one it lost. A marker is a comment, and a comment keeps a formatter from folding the blank lines around it; with the markers gone, a run of blank lines is folded to one, as the formatter would have done. A line a landing holds is never folded, so no run loses its own edges. The count says how many lines were dropped above each line of the marked text, so a landing found there can be read in the text that is written.
fn unmarked(text: &str, tag: &str, landings: &[Place]) -> (String, Vec<usize>) {
    let stripped: Vec<&str> = text
        .lines()
        .filter(|line| mark_at(line, tag).is_none())
        .collect();
    let mut held = vec![false; stripped.len() + 2];
    for (first, last) in landings.iter().flatten() {
        for line in *first..=*last {
            if let Some(slot) = held.get_mut(line) {
                *slot = true;
            }
        }
    }
    let mut kept: Vec<&str> = Vec::with_capacity(stripped.len());
    // `dropped[i]` = lines lost above the marked text's line i, 1-based.
    let mut dropped = vec![0usize; stripped.len() + 2];
    let mut lost = 0usize;
    let mut blank = false;
    for (index, line) in stripped.iter().enumerate() {
        dropped[index + 1] = lost;
        let is_blank = line.trim().is_empty();
        if is_blank && blank && !held[index + 1] {
            lost += 1;
            continue;
        }
        blank = is_blank;
        kept.push(line);
    }
    dropped[stripped.len() + 1] = lost;
    let mut out = kept.join("\n");
    if text.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    (out, dropped)
}

// One landing, read in the written text: both ends lose the lines dropped above them.
fn placed(first: usize, last: usize, dropped: &[usize]) -> (usize, usize) {
    let at = |line: usize| line.saturating_sub(dropped.get(line).copied().unwrap_or(0));
    if last >= first {
        (at(first), at(last))
    } else {
        let spot = at(first);
        (spot, spot - 1)
    }
}

type Key = (String, u64);

// The formatted texts this process has already produced, oldest first.
static MEMO: LazyLock<Mutex<Vec<(Key, Outcome)>>> = LazyLock::new(|| Mutex::new(Vec::new()));

fn remembered(key: &Key) -> Option<Outcome> {
    lock(&MEMO)
        .iter()
        .find(|(held, _)| held == key)
        .map(|(_, outcome)| outcome.clone())
}

fn remember(key: Key, outcome: &Outcome) {
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
