// The Go test support: the tests a batch just wrote, run by name in the package that holds them.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use super::check::{broken, named, position, program, title, unit};
use super::table::shape;
use crate::server::agent::tools::fs::gate;
use crate::server::agent::tools::fs::lang::graph::parts;
use crate::server::agent::tools::fs::lang::verdict::{Fault, Verdict};
use crate::server::agent::tools::fs::lang::{Line, Table, Tests};
use crate::server::platform::stop::Stop;

// What this language owes the tools as test support.
pub(crate) const TESTS: Tests = Tests {
    unit,
    title,
    find,
    run,
};

// The tests this text declares on the given 1-based line ranges, by function name.
fn find(_text: &str, table: &Table, ranges: &[(usize, usize)]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (at, line) in table.pieces() {
        let Some(name) = test_of(table, at, line) else {
            continue;
        };
        let last = table.end(at).unwrap_or(at);
        let hit = ranges
            .iter()
            .any(|(from, to)| at < *to && *from <= last + 1);
        if hit && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

// The test this piece opens. Go marks no test with an attribute: the name and the parameter are the whole declaration, so a helper called `TestThing` that takes no handle is no test.
fn test_of(table: &Table, at: usize, line: &Line) -> Option<String> {
    if shape(line) != Some("func") {
        return None;
    }
    let exact = line.exact.as_deref()?;
    let rest = exact.strip_prefix("func ")?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    let test = name.strip_prefix("Test").is_some_and(|tail| {
        tail.chars()
            .next()
            .is_none_or(|first| first.is_uppercase() || first == '_')
    });
    if !test {
        return None;
    }
    let end = table.end(at).unwrap_or(at);
    let takes = exact.contains("*testing.T") && end >= at;
    takes.then_some(name)
}

// Run exactly these tests in one unit, each package asked for its own names first.
fn run(root: &Path, files: &[PathBuf], names: &[String], stop: &Arc<Stop>) -> Verdict {
    if names.is_empty() {
        return Verdict::Unchecked("no test was named".to_string());
    }
    let mut ran = 0usize;
    let mut faults: Vec<Fault> = Vec::new();
    let mut why: Vec<String> = Vec::new();
    let mut hit: Vec<String> = Vec::new();
    for dir in packages(root, files) {
        match one(root, &dir, names, stop, &mut hit) {
            Verdict::Unchecked(note) => why.push(note),
            Verdict::Passed { ran: count, note } => {
                ran += count;
                if let Some(note) = note {
                    why.push(note);
                }
            }
            Verdict::Failed {
                faults: landed,
                summary,
            } => {
                faults.extend(landed);
                why.push(summary);
            }
        }
    }
    if !faults.is_empty() {
        return Verdict::Failed {
            faults,
            summary: why.join("; "),
        };
    }
    let missing: Vec<String> = names
        .iter()
        .filter(|mine| !hit.contains(mine))
        .cloned()
        .collect();
    let absent = (!missing.is_empty())
        .then(|| format!("the toolchain lists no test named {}", missing.join(", ")));
    if ran > 0 {
        return Verdict::Passed { ran, note: absent };
    }
    Verdict::Unchecked(absent.unwrap_or_else(|| {
        if why.is_empty() {
            "nothing ran".to_string()
        } else {
            why.join("; ")
        }
    }))
}

// The distinct directories these files sit in; a Go test runs a package whole.
fn packages(root: &Path, files: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for file in files {
        let Some(dir) = file.parent() else {
            continue;
        };
        if !dir.starts_with(root) || out.iter().any(|held| held == dir) {
            continue;
        }
        out.push(dir.to_path_buf());
    }
    if out.is_empty() {
        out.push(root.to_path_buf());
    }
    out
}

// One package's turn: list its own names, then run the ones this batch wrote.
fn one(
    root: &Path,
    dir: &Path,
    names: &[String],
    stop: &Arc<Stop>,
    hit: &mut Vec<String>,
) -> Verdict {
    let Some(bin) = program("go") else {
        return Verdict::Unchecked(
            "go was found neither beside this executable nor on PATH".to_string(),
        );
    };
    let mut cmd = Command::new(&bin);
    cmd.args(["test", "-list", ".*"]).arg(".").current_dir(dir);
    let listed = match gate::run(cmd, None, None, stop) {
        Ok(ran) if ran.stopped => return Verdict::Unchecked("the turn was stopped".to_string()),
        Ok(ran) => ran,
        Err(_) => return Verdict::Unchecked("go did not run".to_string()),
    };
    // A package that does not compile says so at this step, and running it would only say it again.
    let refuses = broken(root, dir, &listed.err);
    if !refuses.is_empty() {
        return Verdict::Failed {
            faults: refuses,
            summary: "the tests do not compile".to_string(),
        };
    }
    let known: Vec<String> = listed
        .out
        .lines()
        .map(str::trim)
        .filter(|line| names.iter().any(|mine| mine == line))
        .map(str::to_string)
        .collect();
    for name in &known {
        if !hit.contains(name) {
            hit.push(name.clone());
        }
    }
    if known.is_empty() {
        return Verdict::Unchecked(format!(
            "the toolchain lists no test named {}",
            names.join(", ")
        ));
    }
    let mut cmd = Command::new(&bin);
    cmd.args(["test", "-run", &anchored(&known), "-v", "-count=1"])
        .arg(".")
        .current_dir(dir);
    let ran = match gate::run(cmd, None, None, stop) {
        Ok(ran) if ran.stopped => return Verdict::Unchecked("the turn was stopped".to_string()),
        Ok(ran) => ran,
        Err(_) => return Verdict::Unchecked("go did not run".to_string()),
    };
    outcome(root, dir, &ran.out, &ran.err)
}

// The names as one anchored pattern: `^(TestA|TestB)$`, so a longer name the toolchain also carries never runs.
fn anchored(names: &[String]) -> String {
    format!("^({})$", names.join("|"))
}

// What one `go test -v` run said: how many passed, and one fault per test that failed at the line its own output named.
fn outcome(root: &Path, dir: &Path, out: &str, err: &str) -> Verdict {
    let refuses = broken(root, dir, err);
    if !refuses.is_empty() {
        return Verdict::Failed {
            faults: refuses,
            summary: "the tests do not compile".to_string(),
        };
    }
    let mut passed = 0usize;
    let mut faults: Vec<Fault> = Vec::new();
    let mut open: Option<Block> = None;
    for raw in out.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("=== RUN ") {
            open = Some(Block {
                name: rest.trim().to_string(),
                place: None,
                body: Vec::new(),
            });
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("--- PASS: ") {
            let _ = rest;
            if open.is_some() {
                passed += 1;
                open = None;
            }
            continue;
        }
        if trimmed.starts_with("--- FAIL: ") {
            if let Some(block) = open.take() {
                faults.push(block.fault(root, dir));
            }
            continue;
        }
        if let Some(block) = open.as_mut() {
            // The first place the test's own output names is the line the audit hangs on; the words after it are the message.
            if block.place.is_none()
                && let Some((file, line)) = position(trimmed)
            {
                block.place = Some((named(root, dir, &file), line));
                if let Some((_, said)) = trimmed.split_once(": ") {
                    block.body.push(said.trim().to_string());
                }
                continue;
            }
            if !trimmed.is_empty() {
                block.body.push(trimmed.to_string());
            }
        }
    }
    if !faults.is_empty() {
        let summary = format!("{} failed", faults.len());
        return Verdict::Failed { faults, summary };
    }
    if passed > 0 {
        return Verdict::Passed {
            ran: passed,
            note: None,
        };
    }
    Verdict::Unchecked("nothing ran".to_string())
}

// One test's own output, between its run line and its verdict line.
struct Block {
    name: String,
    // The first place its own output named: `t.Errorf` writes `file:line: words`.
    place: Option<(String, usize)>,
    body: Vec<String>,
}

impl Block {
    // One failed test as a fault the audit can hang on the file it named.
    fn fault(&self, root: &Path, dir: &Path) -> Fault {
        let (file, line) = match &self.place {
            Some((file, line)) => (file.clone(), *line),
            None => (parts::rel(root, dir), 1),
        };
        let said: Vec<&str> = self
            .body
            .iter()
            .map(|line| line.trim())
            .filter(|line| !line.is_empty())
            .take(2)
            .collect();
        let message = if said.is_empty() {
            "the test failed".to_string()
        } else {
            said.join(" ")
        };
        Fault {
            file,
            line,
            message: format!("{}: {message}", self.name),
            helps: Vec::new(),
        }
    }
}
