// The Rust test support: the tests a batch just wrote, run under the names the compiler lists.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use super::check::{cargo, diagnostics, title, unit};
use super::table::shape;
use crate::server::agent::tools::fs::gate;
use crate::server::agent::tools::fs::lang::graph::parts;
use crate::server::agent::tools::fs::lang::verdict::{Fault, Verdict};
use crate::server::agent::tools::fs::lang::{Table, Tests};
use crate::server::platform::stop::Stop;

// What this language owes the tools as test support.
pub(crate) const TESTS: Tests = Tests {
    unit,
    title,
    find,
    run,
};

// The tests this text declares on the given 1-based line ranges, by function name.
fn find(_path: &Path, text: &str, table: &Table, ranges: &[(usize, usize)]) -> Vec<String> {
    let raw: Vec<&str> = text.lines().collect();
    let mut names: Vec<String> = Vec::new();
    for at in 0..raw.len() {
        let Some(name) = test_at(table, &raw, at) else {
            continue;
        };
        // A test's own lines reach from its first attribute to its closing brace.
        let first = block_start(&raw, at);
        let last = table.end(at).unwrap_or(at);
        let hit = ranges
            .iter()
            .any(|(from, to)| first < *to && *from <= last + 1);
        if hit && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

// The test function this line opens, when the attributes above it say so.
fn test_at(table: &Table, raw: &[&str], at: usize) -> Option<String> {
    let line = table.line(at)?;
    if shape(line) != Some("fn") {
        return None;
    }
    let first = block_start(raw, at);
    if !raw[first..at].iter().any(|line| marks_test(line)) {
        return None;
    }
    name_of(line.exact.as_deref()?)
}

// The first line of the attributes above this one, blank and comment lines stepped over; the line itself when none do.
fn block_start(raw: &[&str], at: usize) -> usize {
    let mut first = at;
    let mut probe = at;
    while probe > 0 {
        let above = raw[probe - 1].trim();
        if above.is_empty() || above.starts_with("//") {
            probe -= 1;
            continue;
        }
        if !above.starts_with("#[") {
            break;
        }
        first = probe - 1;
        probe -= 1;
    }
    first
}

// Whether one attribute makes its item a test: `#[test]`, `#[tokio::test]`, `#[actix_rt::test]`.
fn marks_test(line: &str) -> bool {
    let Some(body) = line.trim().strip_prefix("#[") else {
        return false;
    };
    let path = body
        .trim_start_matches(['#', '!'])
        .split(|c: char| c.is_whitespace() || c == '(' || c == ']' || c == '=')
        .next()
        .unwrap_or("");
    path.rsplit("::").next() == Some("test")
}

// The function's own name, out of `pub async fn probe_edge_case<T>()`.
fn name_of(exact: &str) -> Option<String> {
    let mut words = exact.split_whitespace();
    while let Some(word) = words.next() {
        if word != "fn" {
            continue;
        }
        let rest = words.next()?;
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        return (!name.is_empty()).then_some(name);
    }
    None
}

// Run exactly these tests in one unit: the compiler names its own tests, and each is asked for by its full path so nothing else runs.
fn run(root: &Path, files: &[PathBuf], names: &[String], stop: &Arc<Stop>) -> Verdict {
    let mut ran = 0usize;
    let mut faults: Vec<Fault> = Vec::new();
    let mut why: Vec<String> = Vec::new();
    let mut summary = String::new();
    let mut hit: Vec<String> = Vec::new();
    for target in targets(root, files) {
        match one(root, files, &target, names, stop, &mut hit) {
            Verdict::Unchecked(note) => why.push(note),
            Verdict::Passed {
                ran: count,
                note: _,
            } => ran += count,
            Verdict::Failed {
                faults: landed,
                summary: said,
            } => {
                faults.extend(landed);
                summary = said;
            }
        }
    }
    if !faults.is_empty() {
        return Verdict::Failed { faults, summary };
    }
    // A name no target carries is said here, even when another test did run.
    let missing: Vec<String> = names
        .iter()
        .filter(|mine| !hit.contains(mine))
        .cloned()
        .collect();
    let absent = (!missing.is_empty())
        .then(|| format!("the compiler lists no test named {}", missing.join(", ")));
    if ran > 0 {
        return Verdict::Passed { ran, note: absent };
    }
    if let Some(note) = absent {
        return Verdict::Unchecked(note);
    }
    if !summary.is_empty() {
        return Verdict::Unchecked(summary);
    }
    Verdict::Unchecked(if why.is_empty() {
        "nothing ran".to_string()
    } else {
        why.join("; ")
    })
}

// One target's turn: list the compiler's own names, then run the ones this batch wrote.
fn one(
    root: &Path,
    files: &[PathBuf],
    target: &[String],
    names: &[String],
    stop: &Arc<Stop>,
    hit: &mut Vec<String>,
) -> Verdict {
    let mut cmd = Command::new(cargo());
    cmd.arg("test")
        .args(target)
        .arg("--message-format=json")
        .arg("--")
        .arg("--list")
        .current_dir(root);
    // The list step compiles the test target, so a test that does not compile fails here.
    let listed = match gate::run(cmd, None, None, stop) {
        Ok(ran) if ran.stopped => return Verdict::Unchecked("the turn was stopped".to_string()),
        Ok(ran) => ran.out,
        Err(_) => return Verdict::Unchecked("cargo did not run".to_string()),
    };

    let mut fulls: Vec<String> = Vec::new();
    for line in listed.lines() {
        let Some(full) = line.strip_suffix(": test") else {
            continue;
        };
        let full = full.trim();
        if full.is_empty() || !names.iter().any(|mine| tail(full) == mine) {
            continue;
        }
        if !fulls.iter().any(|held| held == full) {
            fulls.push(full.to_string());
        }
    }
    if fulls.is_empty() {
        let (errors, _) = diagnostics(root, &listed);
        if !errors.is_empty() {
            return Verdict::Failed {
                faults: errors,
                summary: "the tests do not compile".to_string(),
            };
        }
        return Verdict::Unchecked(format!(
            "the compiler lists no test named {}",
            names.join(", ")
        ));
    }
    for full in &fulls {
        let name = tail(full).to_string();
        if !hit.contains(&name) {
            hit.push(name);
        }
    }

    let mut cmd = Command::new(cargo());
    cmd.arg("test")
        .args(target)
        .arg("--")
        .arg("--exact")
        .args(&fulls)
        .current_dir(root);
    let run = match gate::run(cmd, None, None, stop) {
        Ok(ran) if ran.stopped => return Verdict::Unchecked("the turn was stopped".to_string()),
        Ok(ran) => ran,
        Err(_) => return Verdict::Unchecked("cargo did not run".to_string()),
    };
    outcome(root, files, &run.out)
}

// What one `cargo test` said: how many passed, and one fault per test that panicked.
fn outcome(root: &Path, files: &[PathBuf], out: &str) -> Verdict {
    let mut passed = 0usize;
    let mut faults: Vec<Fault> = Vec::new();
    let mut summary = String::new();
    let mut block: Option<(String, Vec<String>)> = None;
    let mut trailing = false;
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("---- ") {
            flush(root, files, block.take(), &mut faults);
            trailing = false;
            let name = rest.trim_end_matches(" stdout ----").to_string();
            block = Some((name, Vec::new()));
            continue;
        }
        if let Some(rest) = line.strip_prefix("test result: ") {
            flush(root, files, block.take(), &mut faults);
            trailing = false;
            summary = format!("test result: {rest}");
            continue;
        }
        if line.trim() == "failures:" {
            flush(root, files, block.take(), &mut faults);
            trailing = true;
            continue;
        }
        if trailing {
            continue;
        }
        if line.starts_with("test ") && line.ends_with(" ... ok") {
            passed += 1;
            continue;
        }
        if let Some((_, body)) = block.as_mut() {
            body.push(line.to_string());
        }
    }
    flush(root, files, block.take(), &mut faults);

    if !faults.is_empty() {
        return Verdict::Failed { faults, summary };
    }
    if passed > 0 {
        return Verdict::Passed {
            ran: passed,
            note: None,
        };
    }
    Verdict::Unchecked(if summary.is_empty() {
        "nothing ran".to_string()
    } else {
        format!("{summary} — nothing ran")
    })
}

// One finished block of a failure into a fault.
fn flush(
    root: &Path,
    files: &[PathBuf],
    block: Option<(String, Vec<String>)>,
    faults: &mut Vec<Fault>,
) {
    if let Some((name, body)) = block {
        faults.push(fault_of(root, files, &name, &body));
    }
}

// One failed test as a fault the audit can hang: the panic's own file and line.
fn fault_of(root: &Path, files: &[PathBuf], name: &str, body: &[String]) -> Fault {
    let short = tail(name).to_string();
    let mut message: Vec<String> = Vec::new();
    let mut file = files
        .first()
        .map(|one| parts::rel(root, one))
        .unwrap_or_default();
    let mut line = 1usize;
    for text in body {
        let text = text.trim();
        if text.is_empty() || text.starts_with("note:") {
            continue;
        }
        if let Some(at) = text.find(" panicked at ") {
            let where_ = text[at + " panicked at ".len()..]
                .trim_end_matches(':')
                .trim();
            if let Some((path, number)) = place_of(where_) {
                file = path;
                line = number;
            }
            continue;
        }
        message.push(text.to_string());
    }
    if message.is_empty() {
        message.push("the test failed".to_string());
    }
    Fault {
        file,
        line,
        message: format!("{short}: {}", message.join(" ")),
        helps: Vec::new(),
    }
}

// A panic's `src/lib.rs:20:9` as the file and the line the compiler named.
fn place_of(where_: &str) -> Option<(String, usize)> {
    let mut parts = where_.rsplitn(3, ':');
    parts.next()?;
    let number: usize = parts.next()?.trim().parse().ok()?;
    let path = parts.next()?.trim().trim_start_matches("./");
    (!path.is_empty()).then(|| (path.replace('\\', "/"), number))
}

// The cargo targets these files belong to, one entry per distinct target.
fn targets(root: &Path, files: &[PathBuf]) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    for file in files {
        let args = target_of(root, file);
        if !out.contains(&args) {
            out.push(args);
        }
    }
    if out.is_empty() {
        out.push(Vec::new());
    }
    out
}

// What cargo calls the target one written file belongs to.
fn target_of(root: &Path, file: &Path) -> Vec<String> {
    let rel = file
        .strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/");
    if let Some(rest) = rel.strip_prefix("tests/") {
        return if rest.contains('/') {
            vec!["--tests".to_string()]
        } else {
            vec![
                "--test".to_string(),
                rest.trim_end_matches(".rs").to_string(),
            ]
        };
    }
    if rel == "src/main.rs" || rel.starts_with("src/bin/") {
        return vec!["--bins".to_string()];
    }
    if rel.starts_with("examples/") {
        return vec!["--examples".to_string()];
    }
    if root.join("src/lib.rs").is_file() {
        vec!["--lib".to_string()]
    } else {
        vec!["--bins".to_string()]
    }
}

// The last segment of a full test path: `tests::probe_edge_case` reads `probe_edge_case`.
fn tail(full: &str) -> &str {
    full.rsplit("::").next().unwrap_or(full)
}
