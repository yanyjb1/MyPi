// One line as a report prints it: a base64 payload stands replaced by its decoded size.

use std::borrow::Cow;
use std::fmt::Write as _;

use base64::Engine as _;

use super::sniff;

// A payload shorter than this stays whole: a hash, a token and a short blob are still worth reading.
const LEAST: usize = 512;

// The bytes the longest magic needs to be told: WEBP reads bytes 0..4 and 8..12 under its RIFF head.
const MAGIC: usize = 16;

// The narrowest column a fold takes: a stack of short tokens is not a payload.
const NARROW: usize = 32;

// One line as a report draws it: the text, or a line a folded run took away.
pub(crate) enum Shown<'a> {
    Line(Cow<'a, str>),
    Skip,
}

// One line, with every base64 image payload it carries turned into `base64: N KiB masked`. The rest of the line stays as it read, so a notebook's `"image/png":` still says what the payload was.
pub(crate) fn line(text: &str) -> Cow<'_, str> {
    let bytes = text.as_bytes();
    let mut out: Option<String> = None;
    let mut last = 0usize;
    let mut at = 0usize;
    while at < bytes.len() {
        if !payload_char(bytes[at]) {
            at += 1;
            continue;
        }
        let start = at;
        while at < bytes.len() && payload_char(bytes[at]) {
            at += 1;
        }
        let Some(kib) = masked(&text[start..at]) else {
            continue;
        };
        let buf = out.get_or_insert_with(|| String::with_capacity(text.len()));
        buf.push_str(&text[last..start]);
        let _ = write!(buf, "base64: {kib} KiB masked");
        last = at;
    }
    match out {
        Some(mut buf) => {
            buf.push_str(&text[last..]);
            Cow::Owned(buf)
        }
        None => Cow::Borrowed(text),
    }
}

// Every line of one file, ready to draw: a run of equal-width payload lines folds into its first line, and the lines under it are spent. The caller keeps the physical number of every line it draws, so a fold skips numbers rather than shifting them.
pub(crate) fn block<'a>(lines: &[&'a str]) -> Vec<Shown<'a>> {
    let mut out: Vec<Shown<'a>> = Vec::with_capacity(lines.len());
    let mut at = 0usize;
    while at < lines.len() {
        let stop = run_end(lines, at);
        if stop > at + 1 && joined(lines, at, stop) > LEAST * 2 {
            out.push(Shown::Line(Cow::Owned(folded(lines, at, stop))));
            for _ in at + 1..stop {
                out.push(Shown::Skip);
            }
            at = stop;
            continue;
        }
        out.push(Shown::Line(line(lines[at])));
        at += 1;
    }
    out
}

// How far a folded run reaches from this line; `at + 1` when the line opens none. A run is equal-width payload lines, its last one allowed to be the shorter tail.
fn run_end(lines: &[&str], at: usize) -> usize {
    let Some(text) = payload(lines[at]) else {
        return at + 1;
    };
    let width = text.len();
    if width < NARROW {
        return at + 1;
    }
    let mut end = at + 1;
    while let Some(tail) = lines.get(end).and_then(|line| payload(line)) {
        if tail.len() > width {
            break;
        }
        end += 1;
        if tail.len() < width {
            break;
        }
    }
    end
}

// How many payload characters a range of lines holds.
fn joined(lines: &[&str], from: usize, to: usize) -> usize {
    lines[from..to]
        .iter()
        .filter_map(|line| payload(line))
        .map(str::len)
        .sum()
}

// One folded run's own line: the first line's own indentation, so it still reads as part of what holds it, and how many lines it spent.
fn folded(lines: &[&str], from: usize, to: usize) -> String {
    let indent: String = lines[from]
        .chars()
        .take_while(|one| one.is_whitespace())
        .collect();
    format!(
        "{indent}base64: {} KiB masked ({} lines)",
        size_of(joined(lines, from, to)),
        to - from
    )
}

// One line's own payload, when what the line holds is base64 letters and nothing else. JSON's own decoration around it — the indentation, the quotes a notebook wraps every payload chunk in, the comma that ends all but the last — is not part of the payload.
fn payload(text: &str) -> Option<&str> {
    let bare = text.trim();
    let bare = bare.strip_prefix('"').unwrap_or(bare).trim_end();
    let bare = bare.strip_suffix(',').unwrap_or(bare).trim_end();
    let bare = bare.strip_suffix('"').unwrap_or(bare);
    (!bare.is_empty() && bare.bytes().all(payload_char)).then_some(bare)
}

// The widest drawn run around one line, inside `first..=last`, 0-based and inclusive. A folded run is not context: the run stops where a fold begins, and a change inside a fold is drawn as the one line that stands for the run it fell in.
pub(crate) fn around(
    drawn: &[Shown<'_>],
    first: usize,
    at: usize,
    last: usize,
) -> Option<(usize, usize)> {
    let drawn_at = |n: usize| matches!(drawn.get(n), Some(Shown::Line(_)));
    let last = last.min(drawn.len().saturating_sub(1));
    let at = at.clamp(first, last);
    // A change inside a fold is drawn as the line that stands for the run it fell in, never as nothing.
    let at = (first..=at).rev().find(|n| drawn_at(*n))?;
    let from = (first..=at)
        .rev()
        .take_while(|n| drawn_at(*n))
        .min()
        .unwrap_or(at);
    let to = (at..=last)
        .take_while(|n| drawn_at(*n))
        .last()
        .unwrap_or(at);
    Some((from, to))
}

// What one run of base64 characters hides, in KiB, when it opens with an image's magic. A run of some other bytes, or one short enough to read, hides nothing.
fn masked(run: &str) -> Option<usize> {
    if run.len() <= LEAST {
        return None;
    }
    let head = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(&run.as_bytes()[..MAGIC])
        .ok()?;
    sniff::known(&head)?;
    Some(size_of(run.len()))
}

// How many KiB a run of this many base64 characters decodes to.
fn size_of(chars: usize) -> usize {
    (chars / 4 * 3).div_ceil(1024)
}

// Whether one byte may stand inside a payload's run.
fn payload_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/' || byte == b'='
}
