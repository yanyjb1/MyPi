// The CP check: clang-check reads one file through the project's own compile database, and its diagnostics become the report. No database, no check.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};

use super::facts;
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

// One file's turn may take this long before its tree is killed: a translation unit is smaller than a crate but a header chain is not.
const LIMIT: std::time::Duration = std::time::Duration::from_secs(30);

// The directory holding the compile database this file's project was built with; a project without one is never checked, because no compiler was ever told how to read it.
pub(super) fn unit(path: &Path) -> Option<PathBuf> {
    let root = path.parent()?;
    facts::database(&nearest(root))
}

// The project root above a file: the nearest directory naming a project, or the file's own directory.
fn nearest(dir: &Path) -> PathBuf {
    dir.ancestors()
        .find(|at| facts::MARKS.iter().any(|mark| at.join(mark).is_file()))
        .map_or_else(|| dir.to_path_buf(), Path::to_path_buf)
}

// What a report calls this project: the nearest directory that describes a build, and the database's own directory when none does.
pub(super) fn title(root: &Path) -> String {
    root.ancestors()
        .find(|at| facts::BUILDS.iter().any(|mark| at.join(mark).is_file()))
        .unwrap_or(root)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

// Check the files this batch wrote, one clang-check each. No deadline on the batch as a whole: a write checks once, and only the turn's stop ends it.
fn run(root: &Path, touched: &[PathBuf], stop: &Arc<Stop>) -> Check {
    let Some(bin) = program("clang-check") else {
        return Check::Unchecked(
            "clang-check was found neither beside this executable nor on PATH".to_string(),
        );
    };
    let mut faults: Vec<Fault> = Vec::new();
    let mut warns: Vec<Fault> = Vec::new();
    let mut ran = false;
    for file in touched {
        let mut cmd = Command::new(&bin);
        cmd.arg("-p").arg(root).arg(file);
        let Ok(one) = gate::run(cmd, None, Some(LIMIT), stop) else {
            continue;
        };
        if one.stopped {
            return Check::Unchecked("the turn was stopped".to_string());
        }
        if one.timed_out {
            continue;
        }
        ran = true;
        let (errors, warnings) = diagnostics(root, &one.err);
        faults.extend(errors);
        warns.extend(warnings);
    }
    if !ran {
        return Check::Unchecked("clang-check did not run".to_string());
    }
    faults.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.message == b.message);
    warns.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.message == b.message);
    if faults.is_empty() && warns.is_empty() {
        return Check::Clean;
    }
    Check::Faults { faults, warns }
}

// The errors and the warnings one clang-check run reported. A diagnostic that names no file is dropped: the world's fault is not this batch's.
fn diagnostics(root: &Path, err: &str) -> (Vec<Fault>, Vec<Fault>) {
    let mut errors: Vec<Fault> = Vec::new();
    let mut warns: Vec<Fault> = Vec::new();
    let mut seen: HashSet<(String, usize, String)> = HashSet::new();
    for raw in err.lines() {
        // `path:line:column: error: …`: the file is the whole path, so the line is the segment after it.
        let Some((place, said)) = raw.split_once(": ") else {
            continue;
        };
        let mut head = place.rsplitn(3, ':');
        let (column, line, path) = (head.next(), head.next(), head.next());
        let (Some(_column), Some(line), Some(path)) = (column, line, path) else {
            continue;
        };
        let Ok(line) = line.trim().parse::<usize>() else {
            continue;
        };
        let said = said.trim_start();
        let (list, message) = if let Some(text) = said.strip_prefix("error: ") {
            (&mut errors, text)
        } else if let Some(text) = said.strip_prefix("warning: ") {
            (&mut warns, text)
        } else {
            continue;
        };
        let file = parts::rel(root, &canonical(root, path));
        if !seen.insert((file.clone(), line, message.to_string())) {
            continue;
        }
        list.push(Fault {
            file,
            line,
            message: message.trim().to_string(),
            helps: Vec::new(),
        });
    }
    (errors, warns)
}

// A path clang named, joined to the directory it was run from when it is not absolute.
fn canonical(root: &Path, named: &str) -> PathBuf {
    let path = Path::new(named);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
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
