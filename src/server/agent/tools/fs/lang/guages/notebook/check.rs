// The notebook check: ruff lints the files a batch just wrote, its cell rows turned back into file lines.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};

use super::scan;
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

// The unit a notebook belongs to: the directory it sits in.
fn unit(path: &Path) -> Option<PathBuf> {
    path.parent().map(Path::to_path_buf)
}

// What a report calls that unit.
fn title(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

// Lint the notebooks this batch wrote. No deadline: a batch checks once, and only the turn's stop ends it.
fn run(root: &Path, touched: &[PathBuf], stop: &Arc<Stop>) -> Check {
    let files: Vec<&PathBuf> = touched
        .iter()
        .filter(|path| {
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext == "ipynb")
        })
        .collect();
    if files.is_empty() {
        return Check::Clean;
    }
    let Some(bin) = program("ruff") else {
        return Check::Unchecked(
            "ruff was not found beside this executable or on PATH".to_string(),
        );
    };
    let mut cmd = Command::new(bin);
    cmd.args(["check", "--output-format=json", "--no-cache", "--quiet"])
        .args(&files)
        .current_dir(root);
    let Ok(ran) = gate::run(cmd, None, None, stop) else {
        return Check::Unchecked("ruff did not run".to_string());
    };
    if ran.stopped {
        return Check::Unchecked("the turn was stopped".to_string());
    }
    let Ok(report) = serde_json::from_str::<serde_json::Value>(ran.out.trim()) else {
        // A ruff that found nothing exits 0 and prints an empty list; one that refused prints nothing at all.
        return if ran.code == Some(0) {
            Check::Clean
        } else {
            Check::Unchecked(failure(&ran))
        };
    };
    let mut faults = Vec::new();
    let mut warns = Vec::new();
    let mut cells: HashMap<PathBuf, Vec<scan::Cell>> = HashMap::new();
    for found in report.as_array().into_iter().flatten() {
        let Some(fault) = fault(root, found, &mut cells) else {
            continue;
        };
        // A cell that does not parse is an error whatever the linter calls it; every other finding is a warning, since a linter who called its own findings fatal would stop a batch that only needs to hear them.
        match found.get("code").and_then(|code| code.as_str()) {
            Some("invalid-syntax") => faults.push(fault),
            _ => warns.push(fault),
        }
    }
    faults.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.message == b.message);
    warns.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.message == b.message);
    if faults.is_empty() && warns.is_empty() {
        Check::Clean
    } else {
        Check::Faults { faults, warns }
    }
}

// Why ruff ran and left nothing a reader could use.
fn failure(ran: &gate::Ran) -> String {
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
        Some(line) => format!("ruff exited with {code}: {line}"),
        None => format!("ruff exited with {code} without a word"),
    }
}

// One ruff finding as a fault: its file, the file line its cell row stands on, what it said and how it would mend it. `cells` remembers each file's scan, since one report names the same file many times.
fn fault(
    root: &Path,
    found: &serde_json::Value,
    cells: &mut HashMap<PathBuf, Vec<scan::Cell>>,
) -> Option<Fault> {
    let file = found.get("filename").and_then(|f| f.as_str())?;
    let said = found.get("message").and_then(|m| m.as_str())?;
    let code = found.get("code").and_then(|c| c.as_str());
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
    let path = file_path(root, file);
    let (line, message) = place(&path, found, message, cells);
    Some(Fault {
        file: parts::rel(root, &path),
        line,
        message,
        helps,
    })
}

// One finding's place in the file: a cell's own row turned into the file line it stands on, or the row a plain file names. A cell the scan cannot name leaves the row ruff gave, so no diagnostic is lost to a scan that came up short.
fn place(
    path: &Path,
    found: &serde_json::Value,
    message: String,
    cells: &mut HashMap<PathBuf, Vec<scan::Cell>>,
) -> (usize, String) {
    let row = found
        .get("location")
        .and_then(|spot| spot.get("row"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1) as usize;
    let Some(cell) = found.get("cell").and_then(serde_json::Value::as_u64) else {
        return (row, message);
    };
    let scan = cells
        .entry(path.to_path_buf())
        .or_insert_with(|| scan_of(path));
    let Some(held) = scan.iter().find(|one| one.index == cell as usize) else {
        return (row, message);
    };
    (
        scan::place(held, row),
        format!("cell {} ({}): {message}", held.index, held.kind),
    )
}

// One file's cells, read once and remembered; an unreadable file scans to none.
fn scan_of(path: &Path) -> Vec<scan::Cell> {
    std::fs::read_to_string(path)
        .map(|text| scan::cells(&text))
        .unwrap_or_default()
}

// A ruff filename as a path: absolute as it stands, otherwise against the unit's root.
fn file_path(root: &Path, file: &str) -> PathBuf {
    let named = Path::new(file);
    if named.is_absolute() {
        named.to_path_buf()
    } else {
        root.join(named)
    }
}

// A program of the toolchain: beside this executable first, then whatever PATH holds; `None` when neither has one.
pub(crate) fn program(name: &str) -> Option<PathBuf> {
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
