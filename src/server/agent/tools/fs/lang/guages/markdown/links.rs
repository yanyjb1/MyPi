// The references a Markdown text writes: wikilinks, relative links, and file names in code spans.

use std::path::{Path, PathBuf};

use super::table;
use crate::server::agent::tools::fs::lang;
use crate::server::agent::tools::fs::lang::graph::Loader;
use crate::server::agent::tools::workspace::canonicalize_lenient;

// How a reference was written: it decides what a name that lands nowhere means.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    // `[[note]]`: a note is named by its bare name, and the name is searched.
    Wiki,
    // `[text](path)`: a path relative to this document.
    Link,
    // `` `path/to/file.rs` ``: a file named only to be pointed at, so it must exist.
    Code,
}

// One reference as the text writes it.
pub(super) struct Ref {
    pub(super) line: usize,
    // What the text points at, as written, with a note's suffix dropped.
    pub(super) wrote: String,
    // The heading the reference names after `#`, when it names one.
    pub(super) anchor: Option<String>,
    pub(super) kind: Kind,
}

// Every reference this text writes outside its fenced code.
pub(super) fn read(path: &Path, text: &str) -> Vec<Ref> {
    let mut out: Vec<Ref> = Vec::new();
    let mut fenced: Option<(char, usize)> = None;
    for (at, line) in text.lines().enumerate() {
        if let Some((mark, run)) = fenced {
            if table::closes(line, mark, run) {
                fenced = None;
            }
            continue;
        }
        if let Some((mark, run)) = table::fence(line) {
            fenced = Some((mark, run));
            continue;
        }
        let start = out.len();
        spans(line, at, &mut out);
        // A code span's text is a file name or a specimen of syntax, never a live reference: it is blanked before the two syntax scans read the line.
        let prose = blank(line);
        wikis(&prose, at, &mut out);
        links(&prose, at, &mut out);
        let mut seen: Vec<(String, Kind)> = Vec::new();
        let fresh: Vec<Ref> = out
            .split_off(start)
            .into_iter()
            .filter(|one| {
                let key = (one.wrote.clone(), one.kind);
                !seen.contains(&key) && {
                    seen.push(key);
                    true
                }
            })
            .collect();
        out.extend(fresh);
    }
    // A code span names a file that must be there; the other two may name a note not written yet.
    out.retain(|one| one.kind != Kind::Code || lands(path, &one.wrote).is_some());
    out
}

// The code spans on one line whose text names a file of a kind this build sorts.
// One line with every code span's own text blanked. A backtick with no partner leaves the rest of the line alone.
fn blank(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else {
            break;
        };
        out.push_str(&rest[..open + 1]);
        out.extend(std::iter::repeat_n(' ', close));
        out.push('`');
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

// The code spans on one line whose text names a file of a kind this build sorts.
fn spans(line: &str, at: usize, out: &mut Vec<Ref>) {
    let mut rest = line;
    while let Some(open) = rest.find('`') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('`') else {
            return;
        };
        let inner = &after[..close];
        rest = &after[close + 1..];
        if let Some(wrote) = code_path(inner) {
            out.push(Ref {
                line: at,
                wrote,
                anchor: None,
                kind: Kind::Code,
            });
        }
    }
}

// A code span's own text, when it names a file: no space, a `/` never a `\`, and a suffix a language claims.
fn code_path(inner: &str) -> Option<String> {
    let inner = inner.trim();
    if inner.is_empty() || inner.contains(char::is_whitespace) || inner.contains('\\') {
        return None;
    }
    lang::owner(Path::new(inner))?;
    Some(inner.to_string())
}

// `[[name]]`, `[[name|alias]]`, `[[name#Heading]]` and their embedded form, anywhere on one line.
fn wikis(line: &str, at: usize, out: &mut Vec<Ref>) {
    let mut rest = line;
    while let Some(open) = rest.find("[[") {
        let after = &rest[open + 2..];
        let Some(close) = after.find("]]") else {
            return;
        };
        let inner = &after[..close];
        rest = &after[close + 2..];
        let name = inner.split('|').next().unwrap_or(inner).trim();
        let (wrote, anchor) = split_anchor(name);
        if wrote.is_empty() {
            continue;
        }
        out.push(Ref {
            line: at,
            wrote,
            anchor,
            kind: Kind::Wiki,
        });
    }
}

// `[text](path)`: the target between `](` and the closing paren.
fn links(line: &str, at: usize, out: &mut Vec<Ref>) {
    let chars: Vec<char> = line.chars().collect();
    let mut at_char = 0usize;
    while at_char < chars.len() {
        if chars[at_char] != ']' || chars.get(at_char + 1) != Some(&'(') {
            at_char += 1;
            continue;
        }
        let start = at_char + 2;
        let mut end = start;
        while end < chars.len() && chars[end] != ')' {
            end += 1;
        }
        if end >= chars.len() {
            return;
        }
        let target: String = chars[start..end].iter().collect();
        at_char = end + 1;
        let target = target.trim();
        // A target carrying a title, or a scheme, names no file in this workspace.
        if target.contains(char::is_whitespace) || target.contains("://") {
            continue;
        }
        let (wrote, anchor) = split_anchor(target);
        if wrote.is_empty() {
            continue;
        }
        out.push(Ref {
            line: at,
            wrote,
            anchor,
            kind: Kind::Link,
        });
    }
}

// A reference's path and the heading it names after `#`, with a note's suffix dropped.
fn split_anchor(raw: &str) -> (String, Option<String>) {
    let (path, anchor) = match raw.split_once('#') {
        Some((path, anchor)) => (path, Some(anchor.trim())),
        None => (raw, None),
    };
    let trimmed = path.trim();
    let wrote = trimmed
        .strip_suffix(".md")
        .or_else(|| trimmed.strip_suffix(".markdown"))
        .unwrap_or(trimmed)
        .to_string();
    let anchor = anchor.filter(|one| !one.is_empty()).map(str::to_string);
    (wrote, anchor)
}

// Where a written path lands: read from this document's own directory, with a note's suffix understood when the text leaves it off.
pub(super) fn lands(path: &Path, wrote: &str) -> Option<PathBuf> {
    let dir = path.parent()?;
    let given = dir.join(wrote);
    [
        given.clone(),
        given.with_extension("md"),
        given.with_extension("markdown"),
    ]
    .into_iter()
    .find(|one| one.is_file())
    .map(|one| canonicalize_lenient(&one))
}

// The notes a bare name may mean: every file of this language carrying that stem under the repository, so a name two notes share shows both.
pub(super) fn by_name(loader: &Loader, path: &Path, name: &str) -> Vec<PathBuf> {
    let Some(dir) = path.parent() else {
        return Vec::new();
    };
    let root = match super::facts::domain(dir) {
        Some(dom) => dom.root,
        None => dir.to_path_buf(),
    };
    loader
        .named(&root)
        .find(name)
        .iter()
        .filter(|one| super::facts::sorts(one))
        .cloned()
        .collect()
}
