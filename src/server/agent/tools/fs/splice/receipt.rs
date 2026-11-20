// What the model reads back: the verdict, then one block per edit, shaped like `grep`'s hits — `path:first-last`, the item it sits in, and numbered source. A block that stands there carries `+`; a deletion shows the lines it removed with `-`. The batch's audit hangs its lines where each edit's block ended.

use serde_json::{Value, json};

use super::edit::{Edit, Why};
use super::fmt::Word;
use crate::server::agent::tools::fs::lang::Table;
use crate::server::agent::tools::fs::skeleton;

// How many lines one side of a diff shows before its middle is folded away.
const SIDE: usize = 12;

// One edit of one call, as that call's answer shows it.
pub(crate) struct Row {
    // Which item of the call's `edits`, 1-based; a range cut by a macro carries its owner's.
    pub index: usize,
    // The first line it named, 1-based, in the file as it was before the write.
    pub from: usize,
    pub state: State,
}

pub(crate) enum State {
    // It was written: where it landed, the lines it took and the ones that stand there now.
    Applied {
        first: usize,
        last: usize,
        old: Vec<String>,
        new: Vec<String>,
        // The item holding it, innermost two, outermost first: `impl X for Y > fn z`.
        item: Option<String>,
        // The parser's words, when its block does not parse on its own.
        blame: Option<String>,
    },

    // It lost to an edit the batch had already accepted.
    Overlap,

    // Its anchor words name a line that reads otherwise: the range it asked for, where its own lines sit now, and what it read instead.
    Mismatch {
        sent: (usize, usize),
        now: (usize, usize),
        why: String,
    },

    // The item could not be read at all: no range to name.
    Broken {
        why: String,
    },
}

// The answer's fixed points: the file it writes and the formatter's verdict on the merged text.
pub(crate) struct Sheet<'a> {
    pub path: &'a str,
    // How many lines the file holds now: the one gutter width every block numbers by.
    pub lines: usize,
    pub word: &'a Word,
}

// The answer's text, the memo a front end draws from, and where each edit's audit lines hang.
pub(crate) fn build(sheet: &Sheet<'_>, rows: &[Row]) -> (String, Value, Vec<usize>) {
    let path = sheet.path;
    let lines = sheet.lines;
    let word = sheet.word;
    let width = skeleton::width(lines);
    let landed = rows
        .iter()
        .filter(|row| matches!(row.state, State::Applied { .. }))
        .count();
    // A call that landed whole has nothing to show: the model's next move rests on the file itself, not on a diff it already knows it sent.
    let head = head(path, word, landed, rows.len());
    if landed == rows.len() && !rows.is_empty() && head.ends_with(": OK") {
        return (head, memo(rows), Vec::new());
    }
    let mut out = vec![head];
    let mut marks = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(String::new());
        match &row.state {
            State::Applied {
                first,
                last,
                old,
                new,
                item,
                blame,
            } => {
                out.push(format!("{path}:{}", span(*first, *last)));
                if let Some(item) = item {
                    out.push(format!("  in: {item}"));
                }
                if new.is_empty() {
                    out.extend(side(old, *first, '-', width));
                } else {
                    out.extend(side(new, *first, '+', width));
                }
                if let Some(blame) = blame {
                    out.push(format!("  ! {blame}"));
                }
            }
            State::Overlap => out.push("OVERLAP".to_string()),
            State::Mismatch { sent, now, why } => {
                let moved = if sent == now {
                    String::new()
                } else {
                    "SHIFTED, these lines are now ".to_string()
                };
                out.push(format!(
                    "{moved}{}; REREAD OR REGREP to issue another splice",
                    span(now.0, now.1)
                ));
                out.push(format!("  ! {why}"));
            }
            State::Broken { why } => out.push(format!("  ! {why}")),
        }
        marks.push(out.len());
    }
    if landed > 0 {
        let note = match word {
            Word::Unparsed(why) => Some(format!(
                "[i] fmt · {path} does not parse, written as it stands: {why}"
            )),
            Word::Undecided(why) => Some(format!("[i] fmt · {path} not formatted: {why}")),
            _ => None,
        };
        if let Some(note) = note {
            out.push(String::new());
            out.push(note);
        }
    }
    (out.join("\n"), memo(rows), marks)
}

// A call that never reached its edits: the verdict, then the one reason.
pub(crate) fn refused(path: &str, why: &str) -> String {
    format!("splice {path}: FAILED, NOTHING WRITTEN\n\n! {why}")
}

// A call the user stopped before anything was written.
pub(crate) fn interrupted(path: &str) -> String {
    format!("splice {path}: FAILED, NOTHING WRITTEN\n\n! interrupted: the user stopped the turn")
}

// The verdict line: the file, what became of it, and the formatter's say when all of it landed.
fn head(path: &str, word: &Word, landed: usize, total: usize) -> String {
    let verdict = if landed == 0 {
        "FAILED, NOTHING WRITTEN"
    } else if landed < total {
        "PARTLY FAILED"
    } else {
        match word {
            Word::Plain | Word::Formatted => "OK",
            Word::Unparsed(_) => "WRITTEN, NOT PARSED",
            Word::Undecided(_) => "WRITTEN, UNFORMATTED",
        }
    };
    format!("splice {path}: {verdict}")
}

// `first-last`, or the one line when both are the same.
fn span(first: usize, last: usize) -> String {
    if last > first {
        format!("{first}-{last}")
    } else {
        first.to_string()
    }
}

// One side of the diff, numbered by where its lines stand in the file.
fn side(lines: &[String], first: usize, sign: char, width: usize) -> Vec<String> {
    let rows: Vec<(usize, &str)> = lines
        .iter()
        .enumerate()
        .map(|(at, line)| (first + at, line.as_str()))
        .collect();
    fold(&rows, width, sign)
}

// A side of the diff, its middle folded away once it grows past what one answer may take.
fn fold(rows: &[(usize, &str)], width: usize, sign: char) -> Vec<String> {
    let draw = |at: usize, text: &str| format!("{sign} {at:>width$}|{text}");
    if rows.len() <= SIDE {
        return rows.iter().map(|(at, text)| draw(*at, text)).collect();
    }
    let half = SIDE / 2;
    let mut out: Vec<String> = rows[..half]
        .iter()
        .map(|(at, text)| draw(*at, text))
        .collect();
    out.push(format!("  [+{} lines]", rows.len() - SIDE));
    out.extend(
        rows[rows.len() - half..]
            .iter()
            .map(|(at, text)| draw(*at, text)),
    );
    out
}

// The memo a front end draws from: one segment per edit, in the call's order.
fn memo(rows: &[Row]) -> Value {
    let mut segments = Vec::new();
    for row in rows {
        match &row.state {
            State::Applied {
                first, old, new, ..
            } => segments.push(json!({
                "edit": row.index,
                "state": "applied",
                "fromLine": row.from,
                "atLine": first,
                "old": old.join("\n"),
                "new": new.join("\n"),
            })),
            State::Overlap => segments.push(json!({
                "edit": row.index,
                "state": "rejected",
                "fromLine": row.from,
                "atLine": row.from,
                "error": "OVERLAP",
            })),
            State::Mismatch { now, why, .. } => segments.push(json!({
                "edit": row.index,
                "state": "rejected",
                "fromLine": row.from,
                "atLine": now.0,
                "error": why,
            })),
            State::Broken { why } => segments.push(json!({
                "edit": row.index,
                "state": "rejected",
                "fromLine": 0,
                "atLine": 0,
                "error": why,
            })),
        }
    }
    json!({ "segments": segments })
}

// What the batch knows about one edit before its row is drawn.
pub(crate) struct Fate {
    // Written: where it landed. Refused: where its own lines sit now.
    pub at: (usize, usize),

    // The parser's words, when its block does not parse on its own.
    pub blame: Option<String>,

    // Why it was left out; `None` means it was written.
    pub why: Option<Why>,
}

// One edit, as its own call's answer shows it.
pub(crate) fn row(edit: &Edit, fate: &Fate, lines: &[&str], base: &[String], table: &Table) -> Row {
    let from = edit.start + 1;
    let Some(why) = &fate.why else {
        let (first, last) = fate.at;
        let item = if edit.cut {
            None
        } else {
            item_of(table, first)
        };
        return Row {
            index: edit.index,
            from,
            state: State::Applied {
                first,
                last,
                old: edit.taken(base).to_vec(),
                new: take(lines, first, last),
                item,
                blame: fate.blame.clone(),
            },
        };
    };
    let state = match why {
        Why::Overlap => State::Overlap,
        Why::Mismatch(why) => State::Mismatch {
            sent: (from, edit.end.max(from)),
            now: fate.at,
            why: why.clone(),
        },
    };
    Row {
        index: edit.index,
        from,
        state,
    }
}

// The lines one 1-based inclusive range holds, owned.
fn take(lines: &[&str], first: usize, last: usize) -> Vec<String> {
    if lines.is_empty() || last < first {
        return Vec::new();
    }
    let first = first.clamp(1, lines.len());
    let last = last.min(lines.len());
    if last < first {
        return Vec::new();
    }
    lines[first - 1..last]
        .iter()
        .map(|line| (*line).to_string())
        .collect()
}

// The innermost two items a landing sits in, outermost first.
fn item_of(table: &Table, first: usize) -> Option<String> {
    let chain = table.around(first.saturating_sub(1));
    let tail: Vec<&str> = chain
        .iter()
        .rev()
        .take(2)
        .rev()
        .filter_map(|(_, line)| line.exact.as_deref())
        .collect();
    (!tail.is_empty()).then(|| tail.join(" > "))
}
