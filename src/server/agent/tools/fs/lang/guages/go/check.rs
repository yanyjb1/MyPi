// The Go check: `go build` answers whether each written package compiles, then `go vet` reports what compiles but reads wrong.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};

use crate::platform::stop::Stop;
use crate::server::agent::tools::fs::gate;
use crate::server::agent::tools::fs::lang::Checker;
use crate::server::agent::tools::fs::lang::graph::parts;
use crate::server::agent::tools::fs::lang::verdict::{Check, Fault};

// What this language owes the tools as a checker.
pub(crate) const CHECKER: Checker = Checker {
    unit,
    title,
    shown: parts::rel,
    run,
};

// The module this file belongs to: the nearest directory above it that holds a module file. Go refuses to build outside one, so the unit is also the only place a check may start from.
pub(super) fn unit(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|dir| dir.join("go.mod").is_file())
        .map(Path::to_path_buf)
}

// The module's own path, from the `module` clause; the folder when it names nothing.
pub(super) fn title(root: &Path) -> String {
    let named = std::fs::read_to_string(root.join("go.mod"))
        .ok()
        .and_then(|text| module_path(&text));
    named.unwrap_or_else(|| {
        root.file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default()
    })
}

// Build every package this batch wrote, and only vet what compiled. A batch checks once, and only the turn's stop ends it.
fn run(root: &Path, touched: &[PathBuf], stop: &Arc<Stop>) -> Check {
    let mut built: Vec<Fault> = Vec::new();
    let mut warns: Vec<Fault> = Vec::new();
    let mut ran = false;
    for dir in packages(root, touched) {
        match build(root, &dir, stop) {
            Ran::Quit => return Check::Unchecked("the turn was stopped".to_string()),
            Ran::Gone => return Check::Unchecked(missing()),
            Ran::Done(faults) => {
                ran = true;
                built.extend(faults);
            }
        }
    }
    if !ran {
        return Check::Unchecked("go build did not run".to_string());
    }
    if !built.is_empty() {
        built.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.message == b.message);
        return Check::Faults {
            faults: built,
            warns: Vec::new(),
        };
    }
    for dir in packages(root, touched) {
        match vet(root, &dir, stop) {
            Ran::Quit => return Check::Unchecked("the turn was stopped".to_string()),
            Ran::Gone => break,
            Ran::Done(faults) => warns.extend(faults),
        }
    }
    warns.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.message == b.message);
    if warns.is_empty() {
        return Check::Clean;
    }
    Check::Faults {
        faults: Vec::new(),
        warns,
    }
}

// What one toolchain call answered: its diagnostics, a turn that was stopped, or no program at all.
enum Ran {
    Done(Vec<Fault>),
    Quit,
    Gone,
}

// The distinct directories the written files sit in. A Go package is a directory, so every file of one shares a check.
fn packages(root: &Path, touched: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for file in touched {
        let Some(dir) = file.parent() else {
            continue;
        };
        if !dir.starts_with(root) || out.iter().any(|held| held == dir) {
            continue;
        }
        if builds(dir) {
            out.push(dir.to_path_buf());
        }
    }
    if out.is_empty() {
        out.push(root.to_path_buf());
    }
    out
}

// Whether a directory holds a file the compiler is to build; a package of tests alone holds none.
fn builds(dir: &Path) -> bool {
    let Ok(read) = std::fs::read_dir(dir) else {
        return false;
    };
    read.filter_map(std::result::Result::ok).any(|entry| {
        let name = entry.file_name().to_string_lossy().into_owned();
        name.ends_with(".go") && !name.ends_with("_test.go")
    })
}

// A compile failure that a listing reach, rather than a run, has to report.
pub(super) fn broken(root: &Path, dir: &Path, err: &str) -> Vec<Fault> {
    spoken(root, dir, err)
}

// One package's turn through the compiler. The binary a main package would leave is thrown away: this tool never writes into the tree.
fn build(root: &Path, dir: &Path, stop: &Arc<Stop>) -> Ran {
    let Some(bin) = program("go") else {
        return Ran::Gone;
    };
    let mut cmd = Command::new(bin);
    cmd.args(["build", "-o", discard()])
        .arg(".")
        .current_dir(dir);
    let Ok(ran) = gate::run(cmd, None, None, stop) else {
        return Ran::Gone;
    };
    if ran.stopped {
        return Ran::Quit;
    }
    Ran::Done(spoken(root, dir, &ran.err))
}

// One package's turn through the vetter: its JSON report carries what compiles but reads wrong, and anything on its error stream is a compile failure that reached this far.
fn vet(root: &Path, dir: &Path, stop: &Arc<Stop>) -> Ran {
    let Some(bin) = program("go") else {
        return Ran::Gone;
    };
    let mut cmd = Command::new(bin);
    cmd.args(["vet", "-json"]).arg(".").current_dir(dir);
    let Ok(ran) = gate::run(cmd, None, None, stop) else {
        return Ran::Gone;
    };
    if ran.stopped {
        return Ran::Quit;
    }
    let mut out = spoken(root, dir, &ran.err);
    out.extend(reported(root, dir, &ran.out));
    Ran::Done(out)
}

// What a vetter said in its own report: every finding, at the position it names.
fn reported(root: &Path, dir: &Path, out: &str) -> Vec<Fault> {
    let mut faults = Vec::new();
    let Ok(report) = serde_json::from_str::<serde_json::Value>(out.trim()) else {
        return faults;
    };
    let Some(packages) = report.as_object() else {
        return faults;
    };
    for checks in packages.values().filter_map(serde_json::Value::as_object) {
        for found in checks.values().filter_map(serde_json::Value::as_array) {
            for one in found {
                let Some(place) = one.get("posn").and_then(|p| p.as_str()) else {
                    continue;
                };
                let Some((file, line)) = position(place) else {
                    continue;
                };
                let Some(message) = one.get("message").and_then(|m| m.as_str()) else {
                    continue;
                };
                faults.push(Fault {
                    file: named(root, dir, &file),
                    line,
                    message: message.trim().to_string(),
                    helps: Vec::new(),
                });
            }
        }
    }
    faults
}

// What a toolchain wrote on its error stream: `# pkg` headers dropped, and `path:line:column: words` kept as a fault at its line.
pub(super) fn spoken(root: &Path, dir: &Path, err: &str) -> Vec<Fault> {
    let mut out = Vec::new();
    for line in err.lines() {
        let line = line.trim();
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((file, number)) = position(line) else {
            continue;
        };
        let Some((_, message)) = line.split_once(": ") else {
            continue;
        };
        out.push(Fault {
            file: named(root, dir, &file),
            line: number,
            message: message.trim().to_string(),
            helps: Vec::new(),
        });
    }
    out
}

// The file and the line a `path:line:column: words` row names.
pub(super) fn position(text: &str) -> Option<(String, usize)> {
    let mut parts = text.splitn(3, ':');
    let file = parts.next()?.trim();
    let line = parts.next()?.trim();
    let Ok(line) = line.parse::<usize>() else {
        return None;
    };
    if file.is_empty() || !file.contains('.') {
        return None;
    }
    Some((file.trim_start_matches("./").to_string(), line))
}

// A path a toolchain named, as the unit names it: the tool ran inside the package, so a relative name starts there.
pub(super) fn named(root: &Path, dir: &Path, file: &str) -> String {
    let path = Path::new(file);
    if path.is_absolute() {
        parts::rel(root, path)
    } else {
        parts::rel(root, &dir.join(path))
    }
}

// The module path a `go.mod` declares.
fn module_path(manifest: &str) -> Option<String> {
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with("//") {
            continue;
        }
        let Some(rest) = line.strip_prefix("module") else {
            continue;
        };
        let rest = rest.trim_start();
        if rest.is_empty() {
            continue;
        }
        let value = rest.trim().trim_matches('"').trim_matches('`');
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

// Where a built binary is thrown away, on either platform.
fn discard() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}

// What a batch hears when the toolchain is not installed at all.
fn missing() -> String {
    "go was found neither beside this executable nor on PATH".to_string()
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
