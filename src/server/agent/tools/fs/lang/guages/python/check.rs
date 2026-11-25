// The Python check: ty types the project whole, and ruff lints only the files a batch just wrote.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};

use crate::server::agent::tools::fs::gate;
use crate::server::agent::tools::fs::lang::Checker;
use crate::server::agent::tools::fs::lang::graph::parts;
use crate::server::agent::tools::fs::lang::verdict::{Check, Fault};
use crate::server::platform::stop::Stop;

// What this language owes the tools as a checker.
pub(crate) const CHECKER: Checker = Checker {
    unit,
    title,
    shown: parts::rel,
    run,
};

// A marker that names a project outright.
const STRONG: &[&str] = &["pyproject.toml", "setup.cfg", "setup.py"];

// A marker that hints at a project without claiming one.
const WEAK: &[&str] = &["requirements.txt"];

// The project this file belongs to: the nearest strongly marked directory, then a weakly marked one, then the repository, then its own folder.
pub(super) fn unit(path: &Path) -> Option<PathBuf> {
    let start = path.parent()?;
    let marked = |marks: &[&str]| {
        start
            .ancestors()
            .find(|dir| marks.iter().any(|mark| dir.join(mark).is_file()))
            .map(Path::to_path_buf)
    };
    marked(STRONG)
        .or_else(|| marked(WEAK))
        .or_else(|| {
            start
                .ancestors()
                .find(|dir| dir.join(".git").exists())
                .map(Path::to_path_buf)
        })
        .or_else(|| start.parent().map(Path::to_path_buf))
}

// The project's own name, from `[project]` or `[tool.poetry]`; the folder when unnamed.
pub(super) fn title(root: &Path) -> String {
    std::fs::read_to_string(root.join("pyproject.toml"))
        .ok()
        .and_then(|text| project_name(&text))
        .unwrap_or_else(|| {
            root.file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default()
        })
}

// Type-check one project whole, then lint the files this batch wrote. No deadline: a batch checks once, and only the turn's stop ends it.
fn run(root: &Path, touched: &[PathBuf], stop: &Arc<Stop>) -> Check {
    let ty = typed(root, stop);
    let lint = linted(root, touched, stop);
    match (ty, lint) {
        (Ran::Absent, Ran::Absent) => Check::Unchecked(
            "neither ty nor ruff was found beside this executable or on PATH".to_string(),
        ),
        (typed, linted) => {
            let mut faults = Vec::new();
            let mut warns = Vec::new();
            match typed {
                Ran::Done(mut one, mut other) => {
                    faults.append(&mut one);
                    warns.append(&mut other);
                }
                Ran::Failed(why) => return Check::Unchecked(why),
                Ran::Absent => {}
            }
            match linted {
                Ran::Done(_, mut other) => warns.append(&mut other),
                Ran::Failed(why) => return Check::Unchecked(why),
                Ran::Absent => {}
            }
            faults.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.message == b.message);
            warns.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.message == b.message);
            if faults.is_empty() && warns.is_empty() {
                Check::Clean
            } else {
                Check::Faults { faults, warns }
            }
        }
    }
}

// What one checker had to say: its errors and its warnings, a failure to start, or no program at all.
enum Ran {
    Done(Vec<Fault>, Vec<Fault>),
    Failed(String),
    Absent,
}

// A ty that ran and left no diagnostic behind said something about itself.
fn failure(what: &str, ran: &gate::Ran) -> String {
    let said = ran
        .err
        .lines()
        .chain(ran.out.lines())
        .find(|line| !line.trim().is_empty())
        .map(|line| line.trim().to_string());
    let code = match ran.code {
        Some(code) => code.to_string(),
        None => "a signal".to_string(),
    };
    match said {
        Some(line) => format!("{what} exited with {code}: {line}"),
        None => format!("{what} exited with {code} without a word"),
    }
}

// One project's type diagnostics, from ty's SARIF report.
fn typed(root: &Path, stop: &Arc<Stop>) -> Ran {
    let bin = program("ty");
    let Some(bin) = bin else {
        return Ran::Absent;
    };
    let mut cmd = Command::new(bin);
    cmd.args(["check", "--output-format=sarif", "--no-progress"])
        .arg(".")
        .current_dir(root);
    let Ok(ran) = gate::run(cmd, None, None, stop) else {
        return Ran::Failed("ty did not run".to_string());
    };
    if ran.stopped {
        return Ran::Failed("the turn was stopped".to_string());
    }
    let Ok(report) = serde_json::from_str::<serde_json::Value>(ran.out.trim()) else {
        return Ran::Failed(failure("ty", &ran));
    };
    let mut faults = Vec::new();
    let mut warns = Vec::new();
    let results = report
        .get("runs")
        .and_then(|runs| runs.as_array())
        .and_then(|runs| runs.first())
        .and_then(|run| run.get("results"))
        .and_then(|results| results.as_array());
    for result in results.into_iter().flatten() {
        let Some(fault) = sarif_fault(root, result) else {
            continue;
        };
        match result.get("level").and_then(|level| level.as_str()) {
            Some("error") => faults.push(fault),
            Some("warning") => warns.push(fault),
            _ => {}
        }
    }
    Ran::Done(faults, warns)
}

// One SARIF result as a fault: its file, its line and what it said.
fn sarif_fault(root: &Path, result: &serde_json::Value) -> Option<Fault> {
    let message = result
        .get("message")
        .and_then(|m| m.get("text"))
        .and_then(|t| t.as_str())
        .map(|text| text.trim().to_string())?;
    let place = result
        .get("locations")
        .and_then(|l| l.as_array())
        .and_then(|l| l.first())?;
    let physical = place.get("physicalLocation")?;
    let uri = physical
        .get("artifactLocation")
        .and_then(|a| a.get("uri"))
        .and_then(|u| u.as_str())?;
    let line = physical
        .get("region")
        .and_then(|r| r.get("startLine"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1) as usize;
    let named = place_of(root, uri);
    let mut helps: Vec<String> = Vec::new();
    if let Some(rule) = result.get("ruleId").and_then(|r| r.as_str()) {
        helps.push(rule.to_string());
    }
    Some(Fault {
        file: parts::rel(root, &named),
        line,
        message,
        helps,
    })
}

// A SARIF uri as a path: `file:///a/b.py` or a plain relative name.
fn place_of(root: &Path, uri: &str) -> PathBuf {
    let plain = uri.strip_prefix("file://").unwrap_or(uri);
    let decoded = plain.replace("%20", " ");
    let named = Path::new(&decoded);
    if named.is_absolute() {
        named.to_path_buf()
    } else {
        root.join(named)
    }
}

// The files this batch wrote, linted, from ruff's JSON report. Every finding is a warning: a linter who calls its own findings fatal would stop a batch that only needs to hear them.
fn linted(root: &Path, touched: &[PathBuf], stop: &Arc<Stop>) -> Ran {
    let files: Vec<&PathBuf> = touched
        .iter()
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| matches!(ext, "py" | "pyi"))
        })
        .collect();
    if files.is_empty() {
        return Ran::Done(Vec::new(), Vec::new());
    }
    let Some(bin) = program("ruff") else {
        return Ran::Absent;
    };
    let mut cmd = Command::new(bin);
    cmd.args(["check", "--output-format=json", "--no-cache", "--quiet"])
        .args(&files)
        .current_dir(root);
    let Ok(ran) = gate::run(cmd, None, None, stop) else {
        return Ran::Failed("ruff did not run".to_string());
    };
    if ran.stopped {
        return Ran::Failed("the turn was stopped".to_string());
    }
    let Ok(report) = serde_json::from_str::<serde_json::Value>(ran.out.trim()) else {
        // A ruff that found nothing exits 0 and prints an empty list; one that refused prints nothing at all.
        return if ran.code == Some(0) {
            Ran::Done(Vec::new(), Vec::new())
        } else {
            Ran::Failed(failure("ruff", &ran))
        };
    };
    let mut warns = Vec::new();
    for found in report.as_array().into_iter().flatten() {
        let Some(fault) = ruff_fault(root, found) else {
            continue;
        };
        warns.push(fault);
    }
    Ran::Done(Vec::new(), warns)
}

// One ruff finding as a fault: its file, its line, what it said and how it would mend it.
fn ruff_fault(root: &Path, found: &serde_json::Value) -> Option<Fault> {
    let file = found.get("filename").and_then(|f| f.as_str())?;
    let line = found
        .get("location")
        .and_then(|l| l.get("row"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1) as usize;
    let code = found.get("code").and_then(|c| c.as_str());
    let said = found.get("message").and_then(|m| m.as_str())?;
    let message = match code {
        Some(code) => format!("{code} {said}"),
        None => said.to_string(),
    };
    let mut helps: Vec<String> = Vec::new();
    if let Some(fix) = found
        .get("fix")
        .and_then(|f| f.get("message"))
        .and_then(|m| m.as_str())
    {
        helps.push(fix.to_string());
    }
    Some(Fault {
        file: parts::rel(root, Path::new(file)),
        line,
        message: message.trim().to_string(),
        helps,
    })
}

// The `name` a pyproject declares, under `[project]` first and `[tool.poetry]` second.
fn project_name(manifest: &str) -> Option<String> {
    let mut section = String::new();
    for line in manifest.lines() {
        let line = line.trim();
        if let Some(head) = line.strip_prefix('[')
            && let Some(name) = head.strip_suffix(']')
        {
            section = name.trim().to_string();
            continue;
        }
        if !matches!(section.as_str(), "project" | "tool.poetry") || line.starts_with('#') {
            continue;
        }
        let Some(rest) = line.strip_prefix("name") else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

// A program of the toolchain: beside this executable first, then whatever PATH holds; `None` when neither has one.
pub(super) fn program(name: &str) -> Option<PathBuf> {
    static FOUND: LazyLock<Mutex<HashMap<String, Option<PathBuf>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    if let Some(hit) = lock(&FOUND).get(name) {
        return hit.clone();
    }
    let beside = gate::beside(name);
    let found = beside.is_file().then_some(beside).or_else(|| on_path(name));
    lock(&FOUND).insert(name.to_string(), found.clone());
    found
}

// The one a shell would find first, walking the search path's own directories.
fn on_path(name: &str) -> Option<PathBuf> {
    let dirs = std::env::var_os("PATH")?;
    std::env::split_paths(&dirs)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

// A lock that survives a panicking holder: the state behind it is a cache, never a promise.
fn lock<T>(held: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    held.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
