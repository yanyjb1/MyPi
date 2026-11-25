// The Python test support: the pytest functions a batch just wrote, run by their own node paths.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use super::check::{program, title, unit};
use crate::server::agent::tools::fs::gate;
use crate::server::agent::tools::fs::lang::graph::parts;
use crate::server::agent::tools::fs::lang::verdict::{Fault, Verdict};
use crate::server::agent::tools::fs::lang::{Table, Tests, shape};
use crate::server::platform::stop::Stop;

// What this language owes the tools as test support.
pub(crate) const TESTS: Tests = Tests {
    unit,
    title,
    find,
    run,
};

// The tests this text declares on the given 1-based line ranges, by the node name pytest gives them.
fn find(_text: &str, table: &Table, ranges: &[(usize, usize)]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (at, line) in table.pieces() {
        let Some(called) = test_name(line, shape(table, line)) else {
            continue;
        };
        let last = table.end(at).unwrap_or(at);
        let hit = ranges
            .iter()
            .any(|(from, to)| at < *to && *from <= last + 1);
        if !hit {
            continue;
        }
        let name = match class_of(table, at) {
            Some(class) => format!("{class}::{called}"),
            None => called,
        };
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

// The function name a piece binds, when it is a test.
fn test_name(
    line: &crate::server::agent::tools::fs::lang::Line,
    kind: Option<&str>,
) -> Option<String> {
    if kind != Some("def") {
        return None;
    }
    let exact = line.exact.as_deref()?;
    let rest = exact
        .strip_prefix("async def ")
        .or_else(|| exact.strip_prefix("def "))?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    name.starts_with("test").then_some(name)
}

// The test class this line sits in, when the class's own name begins with `Test`.
fn class_of(table: &Table, at: usize) -> Option<String> {
    for (open, line) in table.pieces() {
        if shape(table, line) != Some("class") || open >= at {
            continue;
        }
        let end = table.end(open)?;
        if end < at {
            continue;
        }
        let exact = line.exact.as_deref()?;
        let rest = exact.strip_prefix("class ")?;
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        return name.starts_with("Test").then_some(name);
    }
    None
}

// Run exactly these tests: pytest answers to a node path, and a parameterized function runs all of its own cases under one name.
fn run(root: &Path, files: &[PathBuf], names: &[String], stop: &Arc<Stop>) -> Verdict {
    if names.is_empty() {
        return Verdict::Unchecked("no test was named".to_string());
    }
    let mut ids: Vec<String> = Vec::new();
    for file in files {
        if !file
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| matches!(ext, "py" | "pyi"))
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let table = crate::server::agent::tools::fs::lang::table(file, &text);
        let known = find(&text, &table, &[(1, text.lines().count().max(1))]);
        let rel = parts::rel(root, file);
        for name in names {
            if known.iter().any(|held| held == name) {
                let id = format!("{rel}::{name}");
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
    }
    if ids.is_empty() {
        return Verdict::Unchecked(format!(
            "the project declares no test named {}",
            names.join(", ")
        ));
    }
    let Some(bin) = program("pytest") else {
        return Verdict::Unchecked(
            "pytest was not found beside this executable or on PATH".to_string(),
        );
    };
    let mut cmd = Command::new(bin);
    cmd.args(&ids)
        .args(["--tb=short", "-q", "--no-header", "-p", "no:cacheprovider"])
        .current_dir(root);
    let ran = match gate::run(cmd, None, None, stop) {
        Ok(ran) if ran.stopped => return Verdict::Unchecked("the turn was stopped".to_string()),
        Ok(ran) => ran,
        Err(_) => return Verdict::Unchecked("pytest did not run".to_string()),
    };
    outcome(root, files, &ran.out, &ran.err)
}

// What one pytest run said: how many passed, and one fault per failed node.
fn outcome(root: &Path, files: &[PathBuf], out: &str, err: &str) -> Verdict {
    let mut faults: Vec<Fault> = Vec::new();
    let mut summary = String::new();
    let mut passed = 0usize;
    let body: Vec<&str> = out.lines().chain(err.lines()).collect();
    // Every `path:line: in name` line a short traceback leaves behind, keyed by the bare function name a `FAILED` row also carries.
    let places = spots(&body);
    for line in &body {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("FAILED ") {
            faults.push(fault_of(root, files, rest, &places));
            continue;
        }
        if line.starts_with("====") && line.contains(" in ") && line.contains('s') {
            summary = line.trim_matches('=').trim().to_string();
        }
    }
    if let Some(at) = summary.find(" passed") {
        let digits: String = summary[..at]
            .chars()
            .rev()
            .take_while(char::is_ascii_digit)
            .collect::<Vec<char>>()
            .into_iter()
            .rev()
            .collect();
        passed = digits.parse().unwrap_or(0);
    }
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

// Every failure's own line, from the `path:line: in name` rows a short traceback prints.
fn spots(body: &[&str]) -> Vec<(String, String, usize)> {
    let mut out: Vec<(String, String, usize)> = Vec::new();
    for line in body {
        let line = line.trim();
        let Some((place, said)) = line.rsplit_once(": in ") else {
            continue;
        };
        let Some(rest) = place.strip_prefix('/').or(Some(place)) else {
            continue;
        };
        let Some((file, number)) = rest.rsplit_once(':') else {
            continue;
        };
        let Ok(number) = number.parse::<usize>() else {
            continue;
        };
        let name = said.split(['(', '[']).next().unwrap_or(said).trim();
        if name.is_empty() {
            continue;
        }
        out.push((file.to_string(), name.to_string(), number));
    }
    out
}

// One `FAILED path::name - message` line as a fault, hung on the file the node names.
fn fault_of(
    root: &Path,
    files: &[PathBuf],
    rest: &str,
    places: &[(String, String, usize)],
) -> Fault {
    let (node, said) = match rest.split_once(" - ") {
        Some((node, said)) => (node.trim(), said.trim().to_string()),
        None => (rest.trim(), "the test failed".to_string()),
    };
    let path = node.split("::").next().unwrap_or(node);
    let named = root.join(path);
    let fallback = files
        .first()
        .map(|one| parts::rel(root, one))
        .unwrap_or_default();
    let file = if named.is_file() {
        parts::rel(root, &named)
    } else {
        fallback
    };
    // A parameterized case carries its own `[id]`; the traceback names the bare function.
    let short = node.rsplit("::").next().unwrap_or(node);
    let bare = short.split('[').next().unwrap_or(short);
    let line = places
        .iter()
        .find(|(_, name, _)| name == bare)
        .map_or(1, |(_, _, line)| *line);
    Fault {
        file,
        line,
        message: format!("{short}: {said}"),
        helps: Vec::new(),
    }
}
