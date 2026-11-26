// What the model reads back: the verdict, then one block per edit, shaped like `grep`'s hits — `path:first-last`, the item it sits in, and numbered source. A block that stands there carries `+`; a deletion shows the lines it removed with `-`. The batch's audit hangs its lines where each edit's block ended.

use serde_json::{Value, json};

use super::edit::{Edit, Why};
use crate::server::agent::tools::fs::lang::Table;
use crate::server::agent::tools::fs::{mask, skeleton};

// How many lines one side of a diff shows before its middle is folded away.
const SIDE: usize = 12;

// What one side of the excerpt holds: its lines, each with the file line it stands on, 1-based.
#[derive(Default)]
pub(crate) struct Run {
    pub lines: Vec<(usize, String)>,
}

// The lines around one edit, taken from the file as it stood before the write.
#[derive(Default)]
pub(crate) struct Context {
    pub before: Option<Run>,
    pub after: Option<Run>,
}

// One edit of one call, as that call's answer shows it and as a front end draws it.
pub(crate) struct Row {
    // Which item of the call's `edits`, 1-based; a range cut by a macro carries its owner's.
    pub index: usize,

    // The first line it named, 1-based, in the file as it was before the write.
    pub from: usize,
    pub state: State,

    // The lines it took, and the lines that stand there now; a refused edit carries the text it sent.
    pub old: Vec<String>,
    pub new: Vec<String>,

    // What stopped it, in the words a front end prints: `OVERLAP`, `MISMATCH: …`, or the reader's own.
    pub error: Option<String>,
    pub context: Context,
}

impl Row {
    // An item the call could not read at all: no range, no lines.
    pub(crate) fn broken(index: usize, why: String) -> Row {
        Row {
            index,
            from: 0,
            state: State::Broken,
            old: Vec::new(),
            new: Vec::new(),
            error: Some(why),
            context: Context::default(),
        }
    }
}

pub(crate) enum State {
    // It was written: where it landed, and the item holding it.
    Applied {
        first: usize,
        last: usize,

        // The item holding it, innermost two, outermost first: `impl X for Y > fn z`.
        item: Option<String>,
    },

    // It lost to an edit the batch had already accepted.
    Overlap,

    // Its anchor words name a line that reads otherwise: the range it asked for, and where its own lines sit now.
    Mismatch {
        sent: (usize, usize),
        now: (usize, usize),
    },

    // Its block does not parse: the range it asked for, and the parser's words.
    Unparsed {
        sent: (usize, usize),
        why: String,
    },

    // The item could not be read at all: no range to name.
    Broken,
}

// The answer's fixed points: the file it writes and how many lines it holds now.
pub(crate) struct Sheet<'a> {
    pub path: &'a str,

    // How many lines the file holds now: the one gutter width every block numbers by.
    pub lines: usize,
}

// The answer's text, the memo a front end draws from, and where each edit's audit lines hang.
pub(crate) fn build(sheet: &Sheet<'_>, rows: &[Row]) -> (String, Value, Vec<usize>) {
    let path = sheet.path;
    let width = skeleton::width(sheet.lines);
    let landed = rows
        .iter()
        .filter(|row| matches!(row.state, State::Applied { .. }))
        .count();

    // A call that landed whole has nothing to show: the model's next move rests on the file itself, not on a diff it already knows it sent.
    let head = head(path, landed, rows.len());

    if landed == rows.len() && !rows.is_empty() && head.ends_with(": OK") {
        return (head, memo(rows), Vec::new());
    }
    let mut out = vec![head];
    let mut marks = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(String::new());
        match &row.state {
            State::Applied { first, last, item } => {
                out.push(format!("{path}:{}", span(*first, *last)));
                if let Some(item) = item {
                    out.push(format!("  in: {item}"));
                }
                if row.new.is_empty() {
                    out.extend(side(&row.old, *first, '-', width));
                } else {
                    out.extend(side(&row.new, *first, '+', width));
                }
            }
            State::Overlap => out.push("OVERLAP".to_string()),
            State::Mismatch { sent, now } => {
                let moved = if sent == now {
                    String::new()
                } else {
                    "SHIFTED, these lines are now ".to_string()
                };
                out.push(format!(
                    "{moved}{}; REREAD OR REGREP to issue another splice",
                    span(now.0, now.1)
                ));
                if let Some(error) = &row.error {
                    out.push(format!("  ! {error}"));
                }
            }
            State::Unparsed { sent, why } => {
                out.push(format!(
                    "{}; the block does not parse, left out and nothing written",
                    span(sent.0, sent.1)
                ));
                out.push(format!("  ! {why}"));
            }
            State::Broken => {
                if let Some(error) = &row.error {
                    out.push(format!("  ! {error}"));
                }
            }
        }
        marks.push(out.len());
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

// The verdict line: the file, and what became of it.
fn head(path: &str, landed: usize, total: usize) -> String {
    let verdict = if landed == 0 {
        "FAILED, NOTHING WRITTEN"
    } else if landed < total {
        "PARTLY FAILED"
    } else {
        "OK"
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
    // Masked where it is drawn: a payload a batch carried through rides in no answer.
    let draw = |at: usize, text: &str| format!("{sign} {at:>width$}|{}", mask::line(text));
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
    let context = trimmed(rows);
    let mut segments = Vec::new();
    for (at, row) in rows.iter().enumerate() {
        let place = match &row.state {
            State::Applied { first, .. } => *first,
            State::Mismatch { now, .. } => now.0,
            State::Unparsed { sent, .. } => sent.0,
            _ => row.from,
        };
        segments.push(json!({
            "edit": row.index,
            "state": match row.state {
                State::Applied { .. } => "applied",
                _ => "rejected",
            },
            "fromLine": row.from,
            "atLine": place,
            "old": masked(&row.old),
            "new": masked(&row.new),
            "error": row.error,
            "before": run_json(&context[at].before),
            "after": run_json(&context[at].after),
        }));
    }
    json!({ "segments": segments })
}

// Every edit's excerpt with what an edit above already drew cut away: a line of the file as it stood is drawn once, and a change reaches down only as far as the next change's first line.
fn trimmed(rows: &[Row]) -> Vec<Context> {
    let mut out: Vec<Context> = (0..rows.len()).map(|_| Context::default()).collect();
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by_key(|at| rows[*at].from);

    // The last line of the file as it stood that the edit just walked has drawn.
    let mut shown = 0usize;
    for (place, at) in order.iter().enumerate() {
        let row = &rows[*at];
        let opens = row.from;
        out[*at].before = row
            .context
            .before
            .as_ref()
            .and_then(|run| between(run, shown, opens.saturating_sub(1)));
        shown = shown.max((opens + row.old.len()).saturating_sub(1));
        let below = order
            .get(place + 1)
            .map_or(usize::MAX, |next| rows[*next].from.saturating_sub(1));
        out[*at].after = row
            .context
            .after
            .as_ref()
            .and_then(|run| between(run, shown, below));
        if let Some(run) = &out[*at].after
            && let Some((last, _)) = run.lines.last()
        {
            shown = shown.max(*last);
        }
    }
    out
}

// One side of an excerpt, cut to the lines `above + 1` through `below`; `None` when it holds none of them. A fold inside that range is drawn as the one line that speaks for it, and the lines it spent are not drawn again.
fn between(run: &Run, above: usize, below: usize) -> Option<Run> {
    let kept: Vec<(usize, String)> = run
        .lines
        .iter()
        .filter(|(at, _)| *at > above && *at <= below)
        .cloned()
        .collect();
    (!kept.is_empty()).then_some(Run { lines: kept })
}

// One side of an excerpt, as the memo carries it; `null` when the edit stands at that edge. Each line carries its own place, so a card that draws them numbers each by its file line rather than by its turn in the list.
fn run_json(run: &Option<Run>) -> Value {
    match run {
        Some(run) => json!({
            "lines": run.lines.iter().map(|(at, text)| json!([at, mask::line(text)])).collect::<Vec<_>>(),
        }),
        None => Value::Null,
    }
}

// Some lines as a memo carries them: every base64 payload stood down to its size, and a folded run of it as the one line that speaks for all of it.
fn masked(lines: &[String]) -> Vec<String> {
    let raw: Vec<&str> = lines.iter().map(String::as_str).collect();
    mask::block(&raw)
        .into_iter()
        .filter_map(|held| match held {
            mask::Shown::Line(text) => Some(text.into_owned()),
            mask::Shown::Skip => None,
        })
        .collect()
}

// What the batch knows about one edit before its row is drawn.
pub(crate) struct Fate {
    // Written: where it landed. Refused: where its own lines sit now.
    pub at: (usize, usize),

    // Why it was left out; `None` means it was written.
    pub why: Option<Why>,
}

// One edit, as its own call's answer shows it.
pub(crate) fn row(
    edit: &Edit,
    fate: &Fate,
    lines: &[&str],
    base: &[String],
    table: &Table,
    base_table: &Table,
) -> Row {
    let from = edit.start + 1;
    let sent = (from, edit.end);
    let context = excerpt(edit, base, base_table);
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
            old: edit.taken(base).to_vec(),
            new: take(lines, first, last),
            error: None,
            context,
            state: State::Applied { first, last, item },
        };
    };
    let (state, error) = match why {
        Why::Overlap => (State::Overlap, "OVERLAP".to_string()),
        Why::Mismatch(why) => (State::Mismatch { sent, now: fate.at }, why.clone()),
        Why::Unparsed(why) => (
            State::Unparsed {
                sent,
                why: why.clone(),
            },
            why.clone(),
        ),
    };
    Row {
        index: edit.index,
        from,
        old: edit.taken(base).to_vec(),
        new: edit.block.clone(),
        error: Some(error),
        context,
        state,
    }
}

// The lines of the item around one edit, taken from the file before the write, with the change's own lines cut out.
fn excerpt(edit: &Edit, base: &[String], table: &Table) -> Context {
    // `edit.end` is one past the last line taken, so the first line after the change is numbered `end + 1`.
    match outermost(table, edit.start) {
        Some((open, end)) => Context {
            before: run_of(base, open + 1, edit.start),
            after: run_of(base, edit.end + 1, end + 1),
        },
        None => Context::default(),
    }
}

// The outermost piece holding a 0-based line, with the line it opens and closes on.
fn outermost(table: &Table, at: usize) -> Option<(usize, usize)> {
    let (open, line) = table.around(at).into_iter().next()?;
    Some((open, line.mate.unwrap_or(open)))
}

// One 1-based inclusive range of the file, when it holds any line: each line with its own place, masked where it is drawn.
fn run_of(base: &[String], first: usize, last: usize) -> Option<Run> {
    if base.is_empty() || last < first {
        return None;
    }
    let first = first.clamp(1, base.len());
    let last = last.min(base.len());
    if last < first {
        return None;
    }
    // A run of payload is drawn as the one line that speaks for it, its own place kept.
    let drawn = mask::block(
        &base[first - 1..last]
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
    let lines = drawn
        .into_iter()
        .enumerate()
        .filter_map(|(back, held)| match held {
            mask::Shown::Line(text) => Some((first + back, text.into_owned())),
            mask::Shown::Skip => None,
        })
        .collect();
    Some(Run { lines })
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
