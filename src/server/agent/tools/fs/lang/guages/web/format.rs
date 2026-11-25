// The web formatter: biome under a deadline, the comment style a run needs by where it stands, and the walk back to the runs a call wrote.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use super::check::{program, shown};
use super::{html, kind};
use crate::server::agent::tools::fs::gate;
use crate::server::agent::tools::fs::lang::verdict::{Draft, Outcome, Place, Unparsed};
use crate::server::platform::stop::Stop;

// How long one formatter round trip may take before its tree is killed.
const LIMIT: Duration = Duration::from_secs(10);

// How many bytes of formatted text this process remembers.
const CACHE: usize = 32 * 1024;

// The word a run's wrapper comment starts with.
const MARK: &str = "mypi:splice";

// How one file kind writes a whole-line comment, and how it writes one that may sit inside a line.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    // `// …`
    Slash,

    // `/* … */`
    Block,

    // `<!-- … -->`
    Tag,
}

// Format one text, or say what stopped it. Only a verdict biome reached is remembered.
pub(crate) fn format(draft: &Draft, path: &Path, stop: &Arc<Stop>) -> Outcome {
    let tag = tag(draft.text);
    let marks = run_marks(draft, path);
    let input = wrapped(draft, &tag, &marks);
    let key = hash(&input);
    if let Some(hit) = remembered(&key) {
        return hit;
    }
    let outcome = formatter(&input, &tag, &marks, path, stop);
    if matches!(outcome, Outcome::Formatted { .. } | Outcome::Unparsed(_)) {
        remember(key, &outcome);
    }
    outcome
}

// The comment style each run needs: a page's own markup takes a tag comment, but a run standing inside its `<script>` or `<style>` takes that block's own syntax, or biome refuses the page whole.
fn run_marks(draft: &Draft, path: &Path) -> Vec<Mark> {
    let fixed = match kind(path) {
        Some("css") => Mark::Block,
        Some("js") => Mark::Slash,
        _ => Mark::Tag,
    };
    if kind(path) != Some("html") {
        return vec![fixed; draft.runs.len()];
    }
    let blocks = html::blocks(&html::scan(draft.text));
    draft
        .runs
        .iter()
        .map(|(first, _)| {
            let at = first.saturating_sub(1);
            let inside = blocks
                .iter()
                // A run opening on the line its close stands on still writes its marker above that line, inside the block.
                .find(|(open, close, _)| at > *open && at <= *close);
            inside.map_or(Mark::Tag, |(_, _, word)| {
                if *word == "script" {
                    Mark::Slash
                } else {
                    Mark::Block
                }
            })
        })
        .collect()
}

// The marker text for one run: the word, this text's fingerprint, the run's index and which edge it is.
fn line(tag: &str, index: usize, edge: &str, mark: Mark) -> String {
    match mark {
        Mark::Slash => format!("// {tag} {index} {edge}"),
        Mark::Block => format!("/* {tag} {index} {edge} */"),
        Mark::Tag => format!("<!-- {tag} {index} {edge} -->"),
    }
}

// The text handed to biome: every run wrapped in its own pair of marker comments, each in the style its own kind reads. A text no run names goes as it is.
fn wrapped(draft: &Draft, tag: &str, marks: &[Mark]) -> String {
    if draft.runs.is_empty() {
        return draft.text.to_string();
    }
    let mut lines: Vec<String> = draft.text.lines().map(str::to_string).collect();
    // Bottom up, so the runs above keep their own numbers while they are still being used.
    for (at, (first, last)) in draft.runs.iter().enumerate().rev() {
        let mark = marks.get(at).copied().unwrap_or(Mark::Tag);
        let open = first.saturating_sub(1).min(lines.len());
        let close = (*last).max(first.saturating_sub(1)).min(lines.len());
        lines.insert(close, line(tag, at, "close", mark));
        lines.insert(open, line(tag, at, "open", mark));
    }
    let mut text = lines.join("\n");
    if draft.text.ends_with('\n') && !text.is_empty() {
        text.push('\n');
    }
    text
}

// One formatter round trip, and the walk back to the runs it was given.
fn formatter(input: &str, tag: &str, marks: &[Mark], path: &Path, stop: &Arc<Stop>) -> Outcome {
    let Some(bin) = program() else {
        return Outcome::Undecided(
            "failed to find `biome`; put it beside this executable or on PATH".to_string(),
        );
    };
    let mut cmd = Command::new(&bin);
    cmd.arg("format")
        .arg(format!("--stdin-file-path={}", shown(path)));
    if kind(path) == Some("html") {
        cmd.arg("--html-formatter-enabled=true");
    }
    match gate::run(cmd, Some(input.as_bytes()), Some(LIMIT), stop) {
        Ok(ran) if ran.stopped => Outcome::Stopped,
        Ok(ran) if ran.timed_out => Outcome::Undecided(format!(
            "biome did not finish in {} s, so it was killed",
            LIMIT.as_secs()
        )),
        Ok(ran) if ran.code == Some(0) && marks.is_empty() => Outcome::Formatted {
            text: ran.out,
            runs: Vec::new(),
        },
        Ok(ran) if ran.code == Some(0) => {
            let found = spots(&ran.out, tag, marks);
            let (text, dropped) = unmarked(&ran.out, tag, marks, &found);
            let runs = found
                .into_iter()
                .map(|place| place.map(|(first, last)| placed(first, last, &dropped)))
                .collect();
            Outcome::Formatted { text, runs }
        }
        Ok(ran) => Outcome::Unparsed(parse(&ran.err)),
        Err(e) => Outcome::Undecided(format!(
            "failed to run `{}`: {e}; put biome beside this executable or on PATH",
            bin.display()
        )),
    }
}

// This run's marker word: the constant, and a fingerprint of the text it marks.
fn tag(base: &str) -> String {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    base.hash(&mut hasher);
    format!("{MARK}:{:016x}", hasher.finish())
}

// Which edge of a run a marker line carries.
enum Edge {
    Open,
    Close,
}

// A marker line's own text, once the comment it is written in is taken off; `None` when the line is no marker of this style.
fn shape(line: &str, mark: Mark) -> Option<&str> {
    let trimmed = line.trim();
    match mark {
        Mark::Slash => trimmed.strip_prefix("// "),
        Mark::Block => trimmed.strip_prefix("/* ")?.strip_suffix(" */"),
        Mark::Tag => trimmed.strip_prefix("<!-- ")?.strip_suffix(" -->"),
    }
}

// The run index and edge this line carries, when it is a marker line, in the style that run was given. A `.html` file may hold markers of more than one style at once, so the line's own wrapper says which run it belongs to.
fn mark_at(line: &str, tag: &str, marks: &[Mark]) -> Option<(usize, Edge)> {
    for mark in [Mark::Slash, Mark::Block, Mark::Tag] {
        let Some(inner) = shape(line, mark) else {
            continue;
        };
        let mut words = inner.split_whitespace();
        if words.next() != Some(tag) {
            continue;
        }
        let Some(index) = words.next().and_then(|word| word.parse().ok()) else {
            continue;
        };
        let edge = match words.next() {
            Some("open") => Edge::Open,
            Some("close") => Edge::Close,
            _ => continue,
        };
        if marks.get(index).copied() == Some(mark) {
            return Some((index, edge));
        }
    }
    None
}

// Where each run's own lines ended up, in the order the runs were handed over. A marker comment takes no line number of its own, so its neighbours are the exact lines the run became. Positions count the text without the marker lines; `unmarked` reads them in the text that is written.
fn spots(text: &str, tag: &str, marks: &[Mark]) -> Vec<Place> {
    let mut blocks: Vec<Place> = Vec::new();
    let mut open: Option<(usize, usize)> = None;
    let mut at = 0usize;
    for line in text.lines() {
        let Some((index, edge)) = mark_at(line, tag, marks) else {
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

// The formatted text without its marker lines, and how many lines before each one it lost. A marker is a comment, and a comment keeps the formatter from folding the blank lines around it; with the markers gone, a run of blank lines is folded to one, as the formatter leaves it elsewhere. A line a landing holds is never folded, so no run loses its own edges. The count says how many lines were dropped above each line of the marked text, so a landing found there can be read in the text that is written.
fn unmarked(text: &str, tag: &str, marks: &[Mark], landings: &[Place]) -> (String, Vec<usize>) {
    let stripped: Vec<&str> = text
        .lines()
        .filter(|line| mark_at(line, tag, marks).is_none())
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

// biome's stderr, cut to the one line and the position a model can act on.
fn parse(stderr: &str) -> Unparsed {
    let mut message: Option<String> = None;
    let mut line = None;
    for raw in stderr.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if line.is_none() {
            line = spot(trimmed);
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
        line,
        message: message.unwrap_or_else(|| "biome refused the text".to_string()),
    }
}

// The line number inside a `stdin:1:7` position, when the sentence carries one.
fn spot(said: &str) -> Option<usize> {
    let chars: Vec<char> = said.chars().collect();
    for at in 0..chars.len() {
        if chars[at] != ':' {
            continue;
        }
        let digits: String = chars[at + 1..]
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if digits.is_empty() || chars.get(at + 1 + digits.len()) != Some(&':') {
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
