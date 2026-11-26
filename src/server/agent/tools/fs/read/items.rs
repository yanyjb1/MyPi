// The item mode: a header the model remembers, answered by every piece whose own text says it.

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::tool::header;
use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::fs::lang::graph::loader;
use crate::server::agent::tools::fs::lang::graph::parts;
use crate::server::agent::tools::fs::lang::{self, Line, Table};
use crate::server::agent::tools::fs::mask;
use crate::server::agent::tools::fs::skeleton;
use crate::server::agent::tools::summary;
use crate::server::agent::tools::workspace::canonicalize_lenient;

// Every piece under `path` whose own text says `wanted`, whole, every line numbered.
pub(crate) fn answer(env: &mut Env, path: &Path, wanted: &str) -> Result<String> {
    let cwd = canonicalize_lenient(&env.workspace.cwd());
    let (text, seen) = render(path, &cwd, wanted)?;
    for (file, body) in &seen {
        env.seen.stamp(file.clone(), body);
    }
    Ok(text)
}

// The answer, and the files whose pieces it drew.
fn render(path: &Path, cwd: &Path, wanted: &str) -> Result<(String, Vec<(PathBuf, String)>)> {
    let needle = summary::flat(wanted);
    anyhow::ensure!(!needle.is_empty(), "`item` names no text");

    let mut out = String::new();
    let mut seen: Vec<(PathBuf, String)> = Vec::new();
    for file in scope(path) {
        let Some(text) = loader::text(&file) else {
            continue;
        };
        let table = lang::table(&file, &text);
        let hits: Vec<usize> = table
            .pieces()
            .into_iter()
            .filter(|(_, line)| says(line, &needle))
            .map(|(at, _)| at)
            .collect();
        if hits.is_empty() {
            continue;
        }
        let raw: Vec<&str> = text.lines().collect();
        let width = skeleton::width(raw.len());
        let drawn = mask::block(&raw);

        // A directory names the file, because one header may stand in several of them.
        if path.is_dir() {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&header(&parts::rel(cwd, &file)));
        }
        for at in hits {
            out.push_str(&block(&raw, &drawn, &table, at, width));
        }
        seen.push((file, text));
    }

    if out.is_empty() {
        return Ok((empty(path, &needle, cwd), seen));
    }
    Ok((out, seen))
}

// The files the call looks through: the one it named, or every sorted file under the directory.
fn scope(path: &Path) -> Vec<PathBuf> {
    if path.is_dir() {
        lang::files(path)
    } else {
        vec![path.to_path_buf()]
    }
}

// One piece whole: its own header with the piece above it, its span, and every line numbered.
fn block(
    raw: &[&str],
    drawn: &[mask::Shown<'_>],
    table: &Table,
    at: usize,
    width: usize,
) -> String {
    let end = table.end(at).unwrap_or(at).min(raw.len().saturating_sub(1));
    let exact = table
        .line(at)
        .and_then(|line| line.exact.as_deref())
        .unwrap_or("");

    let mut out = match outer(table, at) {
        Some(outer) => format!("{exact} ({outer})  {}..{}\n", at + 1, end + 1),
        None => format!("{exact}  {}..{}\n", at + 1, end + 1),
    };
    for index in at..=end {
        let Some(mask::Shown::Line(line)) = drawn.get(index) else {
            continue;
        };
        let line = line.as_ref();
        if line.trim().is_empty() {
            out.push('\n');
        } else {
            out.push_str(&skeleton::numbered(width, Some(index + 1), line));
            out.push('\n');
        }
    }
    out
}

// The piece directly holding one piece, as its own text reads.
fn outer(table: &Table, at: usize) -> Option<&str> {
    let chain = table.around(at);
    let spot = chain.iter().position(|(open, _)| *open == at)?;
    let (_, line) = chain.get(spot.checked_sub(1)?)?;
    line.exact.as_deref()
}

// Whether this piece's own header is exactly what the model named, runs of whitespace aside.
fn says(line: &Line, needle: &str) -> bool {
    line.exact
        .as_deref()
        .map(summary::flat)
        .is_some_and(|exact| exact == needle)
}

// What an answer with nothing in it owes the model.
fn empty(path: &Path, needle: &str, cwd: &Path) -> String {
    let named = parts::rel(cwd, path);
    if path.is_file() && lang::owner(path).is_none() {
        format!("[item] {named}: no sorter reads this file\n")
    } else {
        format!(
            "[item] no piece under {named} opens with the header `{needle}`; copy the piece's own \
             line from an earlier answer, it is not cut short\n"
        )
    }
}
