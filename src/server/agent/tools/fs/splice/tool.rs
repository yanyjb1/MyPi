// The `splice` tool: replace whole lines by number, many ranges in one file.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use serde_json::Value;

use super::edit::{self, Edit, Why};
use super::fmt;
use super::receipt::{self, Fate};
use crate::server::agent::request::contract::{ItemOk, ToolCall};
use crate::server::agent::tools::contract::env::{Answer, Env, License, Shell, Slot};
use crate::server::agent::tools::contract::halt::INTERRUPTED;
use crate::server::agent::tools::contract::output::ToolOutput;

use crate::server::agent::tools::fs::change;
use crate::server::agent::tools::fs::eol;
use crate::server::agent::tools::fs::lang;
use crate::server::agent::tools::fs::lang::graph::parts;
use crate::server::agent::tools::spec::{Args, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;
use crate::server::agent::tools::workspace::write_atomic;
use crate::server::platform::stop::Stop;

const PATH: usize = 0;
const EDITS: usize = 1;
const UNFOLD: usize = 2;

const EDIT_FIELDS: &[Param] = &[
    Param::new(
        "start",
        Kind::Axis,
        "First line of the range, 1-based: `N the words that line opens with`, e.g. `12 pub fn \
         run`. A bare `N` names a blank line and nothing else; on any other line it is refused and \
         nothing is written.",
    ),
    Param::new(
        "end",
        Kind::Axis,
        "Last line of the range, inclusive, in the shape `start` reads: `N the words that line \
         opens with`, then only the first few words of it, never the whole line. Its words are \
         checked when the range covers more than one line, and it is refused when it sits above \
         `start`.",
    ),
    Param::new(
        "new",
        Kind::Text,
        "The replacement; any number of lines. Empty deletes the range. `@@COPY(a..b)@@` fills \
         in lines a..b of the file as it stands before this message; `@@CUT(a..b)@@` fills them \
         in and also removes them from where they were.",
    ),
];

const PARAMS: &[Param] = &[
    Param::new("path", Kind::Text, "File path."),
    Param::new(
        "edits",
        Kind::Objects(EDIT_FIELDS),
        "The ranges to replace, one item each; they may not overlap. Every call on one file in \
         one message runs as one write.",
    ),
    Param::df(
        "unfold",
        Kind::Flag,
        "List every fault of the batch's shared check instead of folding it.",
        "false",
    ),
];

pub(crate) const TOOL: Tool = Tool {
    name: "splice",
    about: "Batch 1-based inclusive line edits in one file. Supports `@@COPY(a..b)@@` / \
            `@@CUT(a..b)@@` line macros. The file's own parser judges the text and leaves out a \
            block that does not parse; the text lands exactly as it was sent, never reformatted. \
            By matched text → `edit`. One check per batch, reported on the last write. The file \
            must have been read or grepped in this conversation; any splice that does not end `OK` \
            takes that authorisation away.",
    fields: "Requires `path` and `edits` (`{start, end, new}`). \
             `start` and `end` read `N the words that line opens with`, e.g. `12 pub fn run`: the \
             line number, then only the first few words of that line, never the whole line. The \
             words confirm the line, and a line that reads otherwise is refused. `start` must not \
             sit above `end`, and both must stand inside the file: a range replaces at least one \
             line. `new: \"\"` deletes the range. `@@COPY(a..b)@@` copies lines `a..b`; \
             `@@CUT(a..b)@@` moves them (auto-deletes `a..b`).\n\
             Edits land in the order sent: one that overlaps an edit already accepted answers \
             `OVERLAP` and the rest of the batch still lands.\n\
             Answer: `splice <path>: <verdict>`, one block per edit, then the batch's check and \
             test reports. A refused edit says where its own lines sit now and that the file must \
             be read or grepped again: a splice that does not end `OK` leaves nothing authorised, \
             so read or grep the file again before the next one.\n\
             `unfold` lists every fault of the batch's check and tests instead of folding it.",
    params: PARAMS,
    run,
    summary: summary::path,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    // A merged run already answered for this call, or the one before it did.
    if let Some(answer) = env.handed.get(&env.current).cloned() {
        return Ok(answer.output());
    }
    let raw = args.str(PATH)?;
    let path = env.workspace.resolve(&raw, true)?;
    let plans = plans(env, &path);
    let license = env.seen.files.get(&path).cloned();
    // What git knew of the file before this batch touches it: a commit that follows must not take the user's own work along.
    let git = crate::git::facts(&path);
    let current = env.current.clone();

    let stop = env.stop.clone();
    let built = env
        .workspace
        .with_write(&path, || compute(&path, license, &plans, &stop))?;
    let built = match built {
        // A sibling of this batch reaches its own turn, finds the stop still set, and says the same thing about the same untouched file: it is said here, never kept for `env.handed`.
        Run::Stopped => {
            return Ok(ToolOutput::text(receipt::interrupted(&raw)).failure(INTERRUPTED));
        }
        // A file never read, one that drifted on disk, or a batch that could not be read whole: it holds no licence, and the next splice reads it first.
        Run::Refused(why) => {
            env.seen.revoke(&path);
            return Ok(ToolOutput::text(receipt::refused(&raw, &why)).failure(why));
        }
        Run::Done(built) => built,
    };
    // The compiler waits for the batch's last write: the audit finishes every written answer then.
    let answers = built.answers(&plans, &path, &raw);
    if built.clean() {
        env.seen.stamp(path.clone(), &built.shown);
    } else {
        env.seen.revoke(&path);
    }

    // The session's ledger holds the file for a commit that follows, with what git knew of it before this batch first touched it.
    if built.written.is_some() {
        let git = env
            .ledger
            .files
            .iter()
            .find(|one| one.path == path)
            .map_or(git, |one| (one.tracked, one.dirty));
        env.ledger.note(path.clone(), git);
    }

    // Every written call hands its shell to the audit: its faults hang under its edits when the batch's check has run, and the crate's report rides on the last written answer. A language that names only tests still enters, so its tests run.
    let audit = built
        .written
        .is_some()
        .then(|| lang::checker(&path))
        .flatten();
    let audit = audit
        .and_then(|checker| {
            let root = (checker.unit)(&path)?;
            let shown = (checker.shown)(&root, &path);
            Some((root, shown))
        })
        .or_else(|| {
            let support = lang::tests(&path)?;
            let root = (support.unit)(&path)?;
            let shown = parts::rel(&root, &path);
            Some((root, shown))
        });
    let mut mine = None;
    for (plan, (answer, slots)) in plans.iter().zip(answers) {
        if let Some((root, shown)) = &audit {
            let unfold = plan
                .args
                .as_ref()
                .ok()
                .and_then(|args| args.flag(UNFOLD).ok())
                .unwrap_or(false);
            env.shells.push(Shell {
                call_id: plan.call_id.clone(),
                mine: shown.clone(),
                root: root.clone(),
                file: path.clone(),
                unfold,
                seam: false,
                slots,
            });
        }
        if plan.call_id == current {
            mine = Some(answer);
        } else {
            env.handed.insert(plan.call_id.clone(), answer);
        }
    }
    match mine {
        Some(answer) => Ok(answer.output()),
        None => Ok(ToolOutput::text(receipt::refused(
            &raw,
            "this call is no longer part of its batch",
        ))),
    }
}

impl Answer {
    // What the front door hands the model.
    fn output(&self) -> ToolOutput {
        let mut out = match &self.memo {
            Some(memo) => ToolOutput::with_memo(self.text.clone(), memo.clone()),
            None => ToolOutput::text(self.text.clone()),
        };
        out.items = self.items.clone();
        if let Some(reason) = &self.refusal {
            out = out.failure(reason.clone());
        }
        out
    }
}

// One splice call of this batch that names the same file.
struct Plan {
    call_id: String,
    // Its arguments, read by the same table the schema is built from.
    args: Result<Args>,
}

// Every splice call of the batch that names `path`, in arrival order.
fn plans(env: &Env, path: &Path) -> Vec<Plan> {
    let mut out = Vec::new();
    for call in &env.batch {
        if call.name != TOOL.name || !names(call, env, path) {
            continue;
        }
        out.push(Plan {
            call_id: call.call_id.clone(),
            args: Args::fill(TOOL.params, &call.args),
        });
    }
    out
}

// Whether this call is one of the batch's, aimed at `path`, before its arguments are trusted.
fn names(call: &ToolCall, env: &Env, path: &Path) -> bool {
    let Ok(value) = serde_json::from_str::<Value>(&call.args) else {
        return false;
    };
    let Some(raw) = value.get("path").and_then(Value::as_str) else {
        return false;
    };
    env.workspace.resolve(raw, true).ok().as_deref() == Some(path)
}

// One item of one call: its slice of the batch's edits, or why it could not be read.
struct Piece {
    // Which item of the call's `edits`, 1-based.
    index: usize,
    from: usize,
    to: usize,
    why: Option<String>,
}

// What one merged run leaves behind: owned, so the compiler can read the crate after the file's lock is gone.
struct Built {
    // The file before the write.
    base: String,

    // The file as it now stands: what was written, or the base when nothing was.
    shown: String,

    // Every edit of the batch, in arrival order, and each call's items.
    edits: Vec<Edit>,
    pieces: Vec<Vec<Piece>>,

    // The edits that were written, as positions into `edits`, and where each one landed.
    held: Vec<usize>,
    landings: Vec<(usize, usize)>,

    // The edits that were left out, as positions into `edits`, each with what stopped it.
    dropped: Vec<(usize, Why)>,

    // The calls whose arguments could not be read at all.
    unreadable: Vec<Option<String>>,

    written: Option<String>,
}

impl Built {
    // A parser that never ran says nothing about the text, so it leaves the licence alone.
    fn clean(&self) -> bool {
        self.dropped.is_empty()
            && self.unreadable.iter().all(Option::is_none)
            && self
                .pieces
                .iter()
                .flatten()
                .all(|piece| piece.why.is_none())
    }

    // One answer per call of the batch: its own edits, its own verdict, and where the audit's check lines will hang.
    // `spots` are line places, `slots` are audit insertion points.
    #[allow(clippy::similar_names)]
    fn answers(&self, plans: &[Plan], path: &Path, shown: &str) -> Vec<(Answer, Vec<Slot>)> {
        let lines: Vec<&str> = self.shown.lines().collect();
        let base = change::lines_of(&self.base);
        let table = lang::table(path, &self.shown);
        let base_table = lang::table(path, &self.base);
        // The file is the unit: one refused range fails every call of the batch on it.
        let refused = !self.dropped.is_empty()
            || self.unreadable.iter().any(Option::is_some)
            || self
                .pieces
                .iter()
                .flatten()
                .any(|piece| piece.why.is_some());
        let items = vec![if refused {
            ItemOk::failed("some ranges were refused; the file is only partly written")
        } else {
            ItemOk::ok()
        }];
        let spots = self.spots();
        let refusal: Vec<Option<Why>> = {
            let mut out: Vec<Option<Why>> = vec![None; self.edits.len()];
            for (index, why) in &self.dropped {
                out[*index] = Some(why.clone());
            }
            out
        };

        (0..plans.len())
            .map(|at| {
                if let Some(why) = &self.unreadable[at] {
                    return (
                        Answer {
                            text: receipt::refused(shown, why),
                            memo: None,
                            items: items.clone(),
                            refusal: Some("the call's ranges could not be read".to_string()),
                        },
                        Vec::new(),
                    );
                }
                let mut rows: Vec<receipt::Row> = Vec::new();
                let mut slots: Vec<Slot> = Vec::new();
                for piece in &self.pieces[at] {
                    if let Some(why) = &piece.why {
                        rows.push(receipt::Row::broken(piece.index, why.clone()));
                        slots.push(Slot {
                            landing: None,
                            insert_at: 0,
                        });
                        continue;
                    }
                    for index in piece.from..piece.to {
                        let why = refusal[index].clone();
                        let fate = Fate {
                            at: spots[index],
                            why: why.clone(),
                        };
                        rows.push(receipt::row(
                            &self.edits[index],
                            &fate,
                            &lines,
                            &base,
                            &table,
                            &base_table,
                        ));
                        slots.push(Slot {
                            landing: why.is_none().then_some(spots[index]),
                            insert_at: 0,
                        });
                    }
                }
                let (text, memo, marks) = receipt::build(
                    &receipt::Sheet {
                        path: shown,
                        lines: self.shown.lines().count(),
                    },
                    &rows,
                );
                // A call that landed whole draws no blocks: the check's lines hang at the end of its verdict line instead of being dropped.
                if marks.is_empty() {
                    let at = text.lines().count();
                    for slot in &mut slots {
                        slot.insert_at = at;
                    }
                } else {
                    for (slot, mark) in slots.iter_mut().zip(marks) {
                        slot.insert_at = mark;
                    }
                }
                (
                    Answer {
                        text,
                        memo: Some(memo),
                        items: items.clone(),
                        refusal: None,
                    },
                    slots,
                )
            })
            .collect()
    }

    // Where every edit's own lines sit in the shown text, written or refused. A refused range shares no line with a written one, so every written range is entirely above it or entirely below; the ones above each add their own change of length, in order, and nothing cancels.
    fn spots(&self) -> Vec<(usize, usize)> {
        let mut out = vec![(0, 0); self.edits.len()];
        for (position, index) in self.held.iter().enumerate() {
            out[*index] = self.landings[position];
        }
        let mut order: Vec<usize> = (0..self.held.len()).collect();
        order.sort_by_key(|at| self.edits[self.held[*at]].start);
        let mut written: Vec<(usize, usize, isize)> = Vec::with_capacity(order.len());
        let mut shift: isize = 0;
        for at in order {
            let edit = &self.edits[self.held[at]];
            let (first, last) = self.landings[at];
            let held = (last + 1).saturating_sub(first);
            shift += held as isize - (edit.end - edit.start) as isize;
            written.push((edit.start, edit.end, shift));
        }
        for (index, _) in &self.dropped {
            let edit = &self.edits[*index];
            let shift = written
                .iter()
                .rfind(|(_, end, _)| *end <= edit.start)
                .map_or(0, |(_, _, shift)| *shift);
            let first = (edit.start as isize + shift).max(0) as usize + 1;
            let last = (edit.end as isize + shift).max(0) as usize;
            out[*index] = (first, last);
        }
        out
    }
}

// What one merged run came to: the file was left alone, the whole group was refused, or it landed.
enum Run {
    Stopped,
    Refused(String),
    Done(Box<Built>),
}

// The whole group: the snapshot, one write, and one answer per call. `Stopped` = the turn was stopped before the write, so the file is exactly as it was and no call has an answer.
fn compute(path: &Path, license: Option<License>, plans: &[Plan], stop: &Arc<Stop>) -> Result<Run> {
    let raw = std::fs::read_to_string(path).context("failed to read file")?;
    // The file's own line ending: the edits are read and laid out on plain lines, and the ending is put back once at the write.
    let ending = eol::Eol::of(&raw);
    let text = ending.plain(&raw).into_owned();
    let base: Vec<&str> = text.lines().collect();

    let Some(license) = license else {
        return Ok(Run::Refused(format!(
            "`{}` is unauthorized: it has not been read in this conversation",
            path.display()
        )));
    };
    if !license.approval {
        return Ok(Run::Refused(format!(
            "`{}` is unauthorized: its last batch did not land whole",
            path.display()
        )));
    }
    if license.hash != License::of(&raw).hash {
        return Ok(Run::Refused(format!(
            "`{}` is unauthorized: it changed on disk after it was read; read or grep it again",
            path.display()
        )));
    }
    let (edits, pieces, unreadable) = flatten(plans, &base);
    let (mut kept, mut dropped) = edit::settle_overlaps(&edits, &base);
    let parser = lang::format(path);

    let mut held: Vec<usize> = Vec::new();
    let mut landings: Vec<(usize, usize)> = Vec::new();
    let mut written: Option<String> = None;

    if !kept.is_empty() {
        let mut picked: Vec<Edit> = kept.iter().map(|at| edits[*at].clone()).collect();
        if let Some(format) = parser {
            let Some(settled) = fmt::settle(&text, &picked, format, path, stop) else {
                return Ok(Run::Stopped);
            };
            // A block the parser cannot parse is left out whole; the rest of the batch still lands.
            if !settled.blamed.is_empty() {
                let mut blames: Vec<Option<String>> = vec![None; picked.len()];
                for (at, why) in settled.blamed {
                    blames[at] = Some(why);
                }
                let mut survivors: Vec<usize> = Vec::with_capacity(picked.len());
                for (at, why) in blames.into_iter().enumerate() {
                    match why {
                        Some(why) => dropped.push((kept[at], Why::Unparsed(why))),
                        None => survivors.push(kept[at]),
                    }
                }
                kept = survivors;
                picked = kept.iter().map(|at| edits[*at].clone()).collect();
            }
        }
        if !kept.is_empty() {
            landings = edit::shifted(&picked);
            let refs: Vec<&Edit> = picked.iter().collect();
            let now = edit::spliced(&text, &refs);
            held = kept;
            if stop.is_stopped() {
                return Ok(Run::Stopped);
            }
            if now != text {
                written = Some(now);
            }
        }
    }

    if let Some(write) = &written {
        write_atomic(path, &ending.write(write))?;
    }
    // What the disk holds now: the written text in the file's own ending, else the file as it was.
    let shown = match &written {
        Some(now) => ending.write(now),
        None => raw.clone(),
    };
    Ok(Run::Done(Box::new(Built {
        base: text,
        shown,
        edits,
        pieces,
        held,
        landings,
        dropped,
        unreadable,
        written,
    })))
}

// The batch's edits in arrival order
type Flattened = (Vec<Edit>, Vec<Vec<Piece>>, Vec<Option<String>>);

fn flatten(plans: &[Plan], base: &[&str]) -> Flattened {
    let mut edits = Vec::new();
    let mut pieces: Vec<Vec<Piece>> = Vec::with_capacity(plans.len());
    let mut unreadable: Vec<Option<String>> = vec![None; plans.len()];

    for (at, plan) in plans.iter().enumerate() {
        let mine = match &plan.args {
            Ok(args) => match args.objects(EDITS) {
                Ok(items) if items.is_empty() => {
                    unreadable[at] = Some("`edits` must name at least one range".to_string());
                    Vec::new()
                }
                Ok(items) => items,
                Err(why) => {
                    unreadable[at] = Some(format!("{why:#}"));
                    Vec::new()
                }
            },
            Err(why) => {
                unreadable[at] = Some(format!("{why:#}"));
                Vec::new()
            }
        };
        let mut rows: Vec<Piece> = Vec::with_capacity(mine.len());

        for (index, read) in edit::call_edits(&mine, base).into_iter().enumerate() {
            let from = edits.len();
            let mut why = None;
            match read {
                Ok(list) => edits.extend(list),
                Err(said) => why = Some(format!("{said:#}")),
            }
            rows.push(Piece {
                index: index + 1,
                from,
                to: edits.len(),
                why,
            });
        }
        pieces.push(rows);
    }
    (edits, pieces, unreadable)
}
