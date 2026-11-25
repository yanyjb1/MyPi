// The answer: the relation's own sections, the items, and the stamps.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use crate::server::agent::tools::contract::env::{GrepLimits, Seen};
use crate::server::agent::tools::fs::lang::graph::parts;
use crate::server::agent::tools::fs::lang::{Relation, Relations, Source};
use crate::server::agent::tools::fs::skeleton;

use super::scan;

// The census's own room: its rows alone, the header and the relation block never charge it.
const LEDGER_MAX: usize = 500;

// One answered call's text, and the files whose lines it showed.
#[allow(clippy::too_many_arguments, clippy::similar_names)]
pub(crate) fn answer(
    query: &str,
    mode: &str,
    files: &[scan::File],
    zone: &scan::Zone,
    caps: GrepLimits,
    lavish: bool,
    misses: &[String],
    scope: &[String],
) -> (String, BTreeSet<usize>) {
    let hits: usize = files
        .iter()
        .flat_map(|file| &file.items)
        .map(|item| item.hits.len())
        .sum();
    let items: usize = files.iter().map(|file| file.items.len()).sum();
    let touched = files.iter().filter(|file| !file.items.is_empty()).count();
    let counted: usize = if items <= caps.deep {
        1
    } else if items <= caps.fold {
        2
    } else {
        3
    };
    // Lavish raises the whole answer one tier: no front, no mixture of two densities.
    let tier = if lavish { counted.max(2) - 1 } else { counted };
    let mut out = format!(
        "grep \"{query}\": {}, {}, {}, {mode}, tier {tier}\n",
        parts::plural(hits, "hit"),
        parts::plural(items, "item"),
        parts::plural(touched, "file"),
    );
    if scope.is_empty() {
        let label = if zone.trees.len() > 3 {
            format!("{} source trees", zone.trees.len())
        } else {
            zone.trees
                .iter()
                .map(|tree| format!("{}/", parts::rel(&zone.root, tree)))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let _ = writeln!(out, "==> {label}");
    } else {
        let _ = writeln!(
            out,
            "==> {}, {} used: {}",
            parts::plural(scope.len(), "path"),
            scope.len() - misses.len(),
            scope.join(" ")
        );
    }

    let mut shown: BTreeSet<usize> = BTreeSet::new();

    // The three sections ask each language's own relation index, over the files that language owns: a workspace mixing Rust, Python and the web answers all of them, never the first one alone.
    let groups = groups(files);
    let many = groups.len() > 1;
    for (relations, members) in groups {
        let sources: Vec<Source> = members
            .iter()
            .map(|at| Source {
                path: &files[*at].path,
                text: &files[*at].text,
                table: &files[*at].table,
            })
            .collect();
        let relation = (relations.scan)(&sources, &zone.root, query);
        // One language alone keeps its own way of saying nothing was found; several together only speak when one of them has something to show.
        if many && relation.used.is_empty() {
            continue;
        }
        for &index in &relation.used {
            if let Some(at) = members.get(index) {
                shown.insert(*at);
            }
        }
        let block = paint(&relation);
        if !block.is_empty() {
            out.push('\n');
            out.push_str(&block);
        }
    }
    out.push('\n');
    let omitted = if tier == 3 {
        let (rows, more) = census(files, zone, &mut shown, LEDGER_MAX);
        out.push_str(&rows);
        more
    } else {
        draw(files, zone, tier, &mut out, &mut shown);
        0
    };
    let tail = footer(tier, lavish, omitted, misses);
    if !tail.is_empty() {
        out.push('\n');
        out.push_str(&tail);
    }
    (out, shown)
}

// The relations of each language this batch holds, with the indices of the files that language owns. One entry per language that names a relation index, in the languages' own order.
fn groups(files: &[scan::File]) -> Vec<(&'static Relations, Vec<usize>)> {
    let mut out: Vec<(&'static Relations, Vec<usize>)> = Vec::new();
    for (at, file) in files.iter().enumerate() {
        let Some(relations) = file.table.language().and_then(|one| one.relations) else {
            continue;
        };
        match out
            .iter_mut()
            .find(|(held, _)| std::ptr::eq(*held, relations))
        {
            Some((_, members)) => members.push(at),
            None => out.push((relations, vec![at])),
        }
    }
    out
}

// What the answer did not give, then the one move that would give more of it.
fn footer(tier: usize, lavish: bool, omitted: usize, misses: &[String]) -> String {
    let mut out = String::new();
    if tier > 1 {
        let hint = if lavish {
            "DOWNSHIFTED: narrow down the `scope`.\n"
        } else {
            "DOWNSHIFTED: set `lavish` true to raise the answer tier, or narrow down the `scope`.\n"
        };
        out.push_str(hint);
    }
    if omitted > 0 {
        let _ = writeln!(out, "[+{omitted} more files]");
    }
    for miss in misses {
        out.push_str(miss);
        out.push('\n');
    }
    out
}

// A relation's rows as one block: each heading, then its lines, in the language's own order.
fn paint(relation: &Relation) -> String {
    let mut out = String::new();
    for row in &relation.rows {
        match &row.heading {
            Some(heading) => out.push_str(heading),
            None => out.push_str(&row.text),
        }
        out.push('\n');
    }
    out
}

// The hits at tier 1 or 2: whole items, then outlines.
fn draw(
    files: &[scan::File],
    zone: &scan::Zone,
    tier: usize,
    out: &mut String,
    shown: &mut BTreeSet<usize>,
) {
    for (at, file) in files.iter().enumerate() {
        if file.items.is_empty() {
            continue;
        }
        let raw: Vec<&str> = file.text.lines().collect();
        let width = skeleton::width(raw.len());
        let label = parts::rel(&zone.root, &file.path);
        let block = if tier == 1 {
            whole(file, &raw, width, &label)
        } else {
            outline(file, &raw, width, &label)
        };
        out.push_str(&block);
        shown.insert(at);
    }
}

// Tier 1: every line of every item; only key lines and hit lines carry a number.
fn whole(file: &scan::File, raw: &[&str], width: usize, label: &str) -> String {
    let mut out = String::new();
    for item in &file.items {
        let _ = writeln!(out, "{label}:{}", item.open + 1);
        let above: Vec<String> = file
            .table
            .around(item.open)
            .iter()
            .filter(|(at, _)| *at != item.open)
            .filter_map(|(at, piece)| {
                let exact = piece.exact.as_deref()?;
                let close = piece.mate.unwrap_or(*at) + 1;
                Some(format!("{exact} ({}..{close})", at + 1))
            })
            .collect();
        if !above.is_empty() {
            let _ = writeln!(out, "  in: {}", above.join(" > "));
        }
        for (index, text) in raw.iter().enumerate().take(item.close + 1).skip(item.open) {
            let mark = (item.hits.contains(&index) || file.table.key(index)).then_some(index + 1);
            out.push_str(&skeleton::numbered(width, mark, text));
            out.push('\n');
        }
    }
    out
}

// Tier 2: the outline of every item — its ancestors' head lines once per file, then its key lines and hit lines, the item's own head row carrying where the piece closes.
fn outline(file: &scan::File, raw: &[&str], width: usize, label: &str) -> String {
    let mut out = format!("{label}\n");
    let mut heads: BTreeSet<usize> = BTreeSet::new();
    for item in &file.items {
        for (at, _) in file.table.around(item.open) {
            if at != item.open && heads.insert(at) {
                out.push_str(&skeleton::numbered(width, Some(at + 1), raw[at]));
                out.push('\n');
            }
        }
        for (index, text) in raw.iter().enumerate().take(item.close + 1).skip(item.open) {
            if !file.table.key(index) && !item.hits.contains(&index) {
                continue;
            }
            out.push_str(&skeleton::numbered(width, Some(index + 1), text));
            out.push('\n');
        }
    }
    out
}

// The census's rows: one per file that holds an item, widest first, within the room, and how many files the room could not hold.
fn census(
    files: &[scan::File],
    zone: &scan::Zone,
    shown: &mut BTreeSet<usize>,
    room: usize,
) -> (String, usize) {
    let mut rows: Vec<(usize, String, usize)> = files
        .iter()
        .enumerate()
        .filter(|(_, file)| !file.items.is_empty())
        .map(|(at, file)| (at, parts::rel(&zone.root, &file.path), file.items.len()))
        .collect();
    rows.sort_by_key(|row| std::cmp::Reverse(row.2));
    let mut out = String::new();
    for (at, path, count) in rows.iter().take(room) {
        let _ = writeln!(out, "  {path}  {}", parts::plural(*count, "item"));
        shown.insert(*at);
    }
    (out, rows.len().saturating_sub(room))
}

// One answered call's stamp: the files whose lines an answer showed, into `seen`.
pub(crate) fn stamp(seen: &mut Seen, files: &[scan::File], shown: &BTreeSet<usize>) {
    for at in shown {
        let file = &files[*at];
        seen.stamp(file.path.clone(), &file.text);
    }
}
