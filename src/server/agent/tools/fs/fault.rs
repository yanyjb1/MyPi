// The batch's shared audit: one check per written unit, run by its own language after the last write. A fault on a written edit hangs under it; the unit's report rides on the last written answer.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::change;
use super::lang::graph::parts;
use super::lang::verdict::{Check, Fault, Unparsed, Verdict};
use super::skeleton;
use crate::server::agent::tools::contract::env::{Env, Shell, Slot};
use crate::server::agent::tools::contract::output::ToolResult;
use crate::server::agent::tools::fs::lang;
use crate::server::agent::tools::fs::lang::{Checker, Table, Tests};
use crate::server::platform::stop::Stop;

// How many diagnostics one folded report lists.
const FAULTS: usize = 12;

// A file that did not parse: the parser's verdict is the whole report.
pub(crate) fn unparsed(why: &Unparsed) -> String {
    let at = match why.line {
        Some(line) => format!(" line {line}:"),
        None => String::new(),
    };
    format!("[i] fmt · not parsed:{at} {}", why.message)
}

// Finish every written call's answer with the batch's one check.
pub(crate) fn settle(env: &mut Env, results: &mut [ToolResult]) {
    let stop = env.stop.clone();
    let shells = std::mem::take(&mut env.shells);
    if shells.is_empty() {
        return;
    }
    if stop.is_stopped() {
        let note = [
            "[i] check · interrupted".to_string(),
            "[i] test · interrupted".to_string(),
        ];
        for shell in &shells {
            if wrote(shell) {
                append(results, &shell.call_id, &note);
            }
        }
        return;
    }
    let checks = gather(&shells, &stop);
    let tests = gather_tests(&shells, &checks, &stop);
    finish(&shells, &checks, &tests, results);
}

// One check per unit, in the order the writes named them: each unit's own checker, and the files this batch wrote into it.
fn gather(shells: &[Shell], stop: &Arc<Stop>) -> Vec<(PathBuf, Check, &'static Checker)> {
    let mut units: Vec<(PathBuf, &'static Checker, Vec<PathBuf>)> = Vec::new();
    for shell in shells {
        let Some(checker) = lang::checker(&shell.file) else {
            continue;
        };
        match units.iter_mut().find(|(root, ..)| *root == shell.root) {
            Some((_, _, files)) => files.push(shell.file.clone()),
            None => units.push((shell.root.clone(), checker, vec![shell.file.clone()])),
        }
    }
    units
        .into_iter()
        .map(|(root, checker, files)| {
            let check = (checker.run)(&root, &files, stop);
            (root, check, checker)
        })
        .collect()
}

// One test run per unit whose writes declared a test, in the order the writes named them. A unit whose check found an error is not run: the same compile error twice reads as two mistakes.
fn gather_tests(
    shells: &[Shell],
    checks: &[(PathBuf, Check, &'static Checker)],
    stop: &Arc<Stop>,
) -> Vec<(PathBuf, &'static Tests, Verdict)> {
    let mut units: Vec<(PathBuf, &'static Tests, Vec<PathBuf>, Vec<String>)> = Vec::new();
    for shell in shells {
        let Some(support) = lang::tests(&shell.file) else {
            continue;
        };
        let Some(root) = (support.unit)(&shell.file) else {
            continue;
        };
        let Ok(text) = std::fs::read_to_string(&shell.file) else {
            continue;
        };
        let table = lang::table(&shell.file, &text);
        let mut ranges: Vec<(usize, usize)> =
            shell.slots.iter().filter_map(|slot| slot.landing).collect();
        // A whole-file write has no landing; a splice that wrote nothing has no tests to run.
        if ranges.is_empty() {
            if !shell.seam {
                continue;
            }
            ranges.push((1, text.lines().count().max(1)));
        }
        let found = (support.find)(&text, &table, &ranges);
        if found.is_empty() {
            continue;
        }
        match units.iter_mut().find(|(held, ..)| *held == root) {
            Some((_, _, files, names)) => {
                files.push(shell.file.clone());
                for name in found {
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
            }
            None => units.push((root, support, vec![shell.file.clone()], found)),
        }
    }
    units
        .into_iter()
        .map(|(root, support, files, names)| {
            let broken = checks.iter().any(|(held, check, _)| {
                *held == root && matches!(check, Check::Faults { faults, .. } if !faults.is_empty())
            });
            let verdict = if broken {
                Verdict::Unchecked("check did not pass".to_string())
            } else {
                (support.run)(&root, &files, &names, stop)
            };
            (root, support, verdict)
        })
        .collect()
}

// The audit's pure half: every diagnostic lands once — under the edit whose lines it hit, under the write that replaced the whole file, or in the crate's own list.
pub(crate) fn finish(
    shells: &[Shell],
    checks: &[(PathBuf, Check, &'static Checker)],
    tests: &[(PathBuf, &'static Tests, Verdict)],
    results: &mut [ToolResult],
) {
    let unfold = shells.iter().any(|shell| shell.unfold);
    let mut report: Vec<String> = Vec::new();
    // Tests and checks share one insert pass so every diagnostic keeps its line number.
    let mut inserts: Vec<(String, usize, Vec<String>)> = Vec::new();

    // Tests come first: a failure outranks a warning when both hang under the same edit.
    let mut covered: BTreeMap<PathBuf, (String, Table)> = BTreeMap::new();
    let mut tested: Vec<String> = Vec::new();
    for (root, support, verdict) in tests {
        let name = (support.title)(root);
        match verdict {
            Verdict::Unchecked(why) => {
                tested.push(format!("[i] test · {name} not run: {why}"));
            }
            Verdict::Passed { ran, note } => {
                tested.push(format!("[i] test · {name} {ran} passed"));
                if let Some(note) = note {
                    tested.push(format!("[i] test · {name}: {note}"));
                }
            }
            Verdict::Failed {
                faults,
                summary: said,
            } => {
                let mut lones: Vec<Fault> = Vec::new();
                for fault in faults {
                    place(root, shells, fault, &mut inserts, &mut lones, &mut covered);
                }
                tested.push(format!("[i] test · {name}: {said}"));
                tested.extend(listing(&lones, &[], unfold));
            }
        }
    }

    for (root, check, checker) in checks {
        let name = (checker.title)(root);
        let (faults, warns) = match check {
            Check::Clean => {
                report.push(format!("[i] check · {name}: clean"));
                continue;
            }
            Check::Unchecked(why) => {
                report.push(format!("[i] check · {name}: not type-checked: {why}"));
                continue;
            }
            Check::Faults { faults, warns } => (faults, warns),
        };

        let mut lones: (Vec<Fault>, Vec<Fault>) = (Vec::new(), Vec::new());
        for fault in faults {
            place(
                root,
                shells,
                fault,
                &mut inserts,
                &mut lones.0,
                &mut covered,
            );
        }
        for fault in warns {
            place(
                root,
                shells,
                fault,
                &mut inserts,
                &mut lones.1,
                &mut covered,
            );
        }

        report.push(head(&name, faults.len(), warns.len()));
        report.extend(listing(&lones.0, &lones.1, unfold));
    }

    // A later insert point first: the earlier ones keep their line numbers.
    inserts.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    for (call_id, at, lines) in &inserts {
        insert(results, call_id, *at, lines);
    }

    report.extend(tested);

    // What the batch still owes the reader rides on the last written answer.
    if let Some(holder) = shells
        .iter()
        .rev()
        .find(|shell| wrote(shell) && holds(results, &shell.call_id))
    {
        append(results, &holder.call_id, &report);
    }
}

// Whether one call actually left lines in the file.
fn wrote(shell: &Shell) -> bool {
    shell.seam || shell.slots.iter().any(|slot| slot.landing.is_some())
}

// Whether one call's answer can still take lines.
fn holds(results: &[ToolResult], call_id: &str) -> bool {
    results
        .iter()
        .any(|result| result.call_id == call_id && result.reason.is_none())
}

// Where one diagnostic belongs: under the edit whose lines it landed on, under the write that replaced the whole file, or nowhere a shell can hold it.
fn place(
    root: &Path,
    shells: &[Shell],
    fault: &Fault,
    groups: &mut Vec<(String, usize, Vec<String>)>,
    lone: &mut Vec<Fault>,
    covered: &mut BTreeMap<PathBuf, (String, Table)>,
) {
    let mine = || {
        shells
            .iter()
            .filter(move |shell| shell.root == *root && shell.mine == fault.file)
    };
    if let Some((call_id, at)) = mine().find_map(|shell| {
        let slot = shell.slots.iter().find(|slot| covers(slot, fault.line))?;
        Some((shell.call_id.clone(), slot.insert_at))
    }) {
        group(groups, &call_id, at, vec![hanging(fault)]);
        return;
    }
    if let Some(shell) = mine().rev().find(|shell| shell.seam) {
        let at = shell.slots.first().map_or(0, |slot| slot.insert_at);
        group(groups, &shell.call_id, at, seam_block(root, fault, covered));
        return;
    }
    lone.push(fault.clone());
}

// The report's head: how many of each the crate's own check found.
fn head(name: &str, faults: usize, warns: usize) -> String {
    let mut counts: Vec<String> = Vec::new();
    if faults > 0 {
        counts.push(parts::plural(faults, "error"));
    }
    if warns > 0 {
        counts.push(parts::plural(warns, "warn"));
    }
    format!("[i] check · {name}: {}", counts.join(", "))
}

// The report's lines: every error, then every warning; folded past the cap unless unfolded.
fn listing(faults: &[Fault], warns: &[Fault], unfold: bool) -> Vec<String> {
    let lines: Vec<String> = faults.iter().chain(warns.iter()).map(one).collect();
    if unfold || lines.len() <= FAULTS {
        return lines;
    }
    let mut out: Vec<String> = lines[..FAULTS].to_vec();
    out.push(format!(
        "[+{} more faults: set `unfold` true to list them]",
        lines.len() - FAULTS
    ));
    out
}

// One report line: where, what, and the compiler's own suggestion.
fn one(fault: &Fault) -> String {
    let mut line = format!("  {}:{}: {}", fault.file, fault.line, fault.message);
    if !fault.helps.is_empty() {
        let _ = write!(line, " | help: {}", fault.helps.join("; "));
    }
    line
}

// One fault's line as it hangs under an edit of a spliced file.
fn hanging(fault: &Fault) -> String {
    let mut line = format!("  ! line {}: {}", fault.line, fault.message);
    if !fault.helps.is_empty() {
        let _ = write!(line, " | help: {}", fault.helps.join("; "));
    }
    line
}

// One fault's lines as they stand under a written file's answer: the source seam and the words.
fn seam_block(
    root: &Path,
    fault: &Fault,
    covered: &mut BTreeMap<PathBuf, (String, Table)>,
) -> Vec<String> {
    let mut out = vec![format!("--- {}:{} ---", fault.file, fault.line)];
    let full = root.join(&fault.file);
    let (source, table) = covered.entry(full.clone()).or_insert_with(|| {
        let text = std::fs::read_to_string(&full).unwrap_or_default();
        let table = lang::table(&full, &text);
        (text, table)
    });
    out.extend(seam(source, table, fault.line));
    for help in &fault.helps {
        out.push(format!("! {help}"));
    }
    out
}

// Whether a fault's line lands inside one edit's landing.
fn covers(slot: &Slot, line: usize) -> bool {
    matches!(slot.landing, Some((first, last)) if first <= line && line <= last)
}

// Collect one insert point's lines, in the order the compiler named them.
fn group(
    groups: &mut Vec<(String, usize, Vec<String>)>,
    call_id: &str,
    at: usize,
    lines: Vec<String>,
) {
    match groups
        .iter_mut()
        .find(|(call, held, _)| call.as_str() == call_id && *held == at)
    {
        Some((_, _, held)) => held.extend(lines),
        None => groups.push((call_id.to_string(), at, lines)),
    }
}

// The answer one call still owns; a refused or missing one takes nothing.
fn slot<'a>(results: &'a mut [ToolResult], call_id: &str) -> Option<&'a mut ToolResult> {
    results
        .iter_mut()
        .find(|result| result.call_id == call_id && result.reason.is_none())
}

// Insert whole lines into one answer's text; a refused or missing answer stays as it is.
fn insert(results: &mut [ToolResult], call_id: &str, at: usize, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let Some(result) = slot(results, call_id) else {
        return;
    };
    let mut text = change::lines_of(&result.result);
    let at = at.min(text.len());
    for (offset, line) in lines.iter().enumerate() {
        text.insert(at + offset, line.clone());
    }
    result.result = text.join("\n");
}

// Append whole lines to one answer's text, on the same terms.
fn append(results: &mut [ToolResult], call_id: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let Some(result) = slot(results, call_id) else {
        return;
    };
    let mut text = result.result.clone();
    text.push('\n');
    for line in lines {
        text.push('\n');
        text.push_str(line);
    }
    result.result = text;
}

// The error's own lines, two above and two below, and the piece they sit in.
fn seam(text: &str, table: &Table, line: usize) -> Vec<String> {
    let raw: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let width = skeleton::width(raw.len());
    let spot = line.saturating_sub(1);
    if let Some(block) = skeleton::innermost(table, spot) {
        out.push(format!(
            "in: {} ({}..{})",
            block.head,
            block.start + 1,
            block.end + 1
        ));
    }
    out.extend(skeleton::seam(&raw, line, line, width));
    out
}
