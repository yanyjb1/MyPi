// The web check: biome lints the files a batch wrote, inside the page's own root.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, LazyLock, Mutex};

use super::facts;
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

// The page root this file belongs to.
fn unit(path: &Path) -> Option<PathBuf> {
    (facts::HUB.domain?)(path).map(|dom| dom.root)
}

// What a report calls that root.
fn title(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "web".to_string())
}

// The files this batch wrote, linted together, from biome's JSON report. Every finding is a warning: a linter that called its own findings fatal would stop a batch that only needs to hear them.
fn run(root: &Path, touched: &[PathBuf], stop: &Arc<Stop>) -> Check {
    let files: Vec<&PathBuf> = touched
        .iter()
        .filter(|path| super::kind(path).is_some())
        .collect();
    if files.is_empty() {
        return Check::Clean;
    }
    let Some(bin) = program() else {
        return Check::Unchecked(
            "failed to find `biome`; put it beside this executable or on PATH".to_string(),
        );
    };
    let mut warns: Vec<Fault> = Vec::new();
    for Pass { cmd, input, rename } in passes(&bin, root, &files) {
        match report(cmd, input.as_deref(), root, stop, rename.as_deref()) {
            Ok(mut found) => warns.append(&mut found),
            Err(why) => return Check::Unchecked(why),
        }
    }
    warns.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.message == b.message);
    if warns.is_empty() {
        Check::Clean
    } else {
        Check::Faults {
            faults: Vec::new(),
            warns,
        }
    }
}

// One lint pass: the command, the text it reads on stdin when it takes one, and the file a report about that text belongs to.
struct Pass {
    cmd: Command,
    input: Option<Vec<u8>>,
    rename: Option<PathBuf>,
}

// The passes one batch needs. `.htm` is a name biome does not know, so a page written with it is handed over on stdin under the `.html` name biome does read.
fn passes(bin: &Path, root: &Path, files: &[&PathBuf]) -> Vec<Pass> {
    let mut out = Vec::new();
    let mut plain: Vec<&PathBuf> = Vec::new();
    for path in files {
        if path.extension().and_then(|e| e.to_str()) != Some("htm") {
            plain.push(path);
            continue;
        }
        let Some(text) = std::fs::read_to_string(path).ok() else {
            continue;
        };
        let mut cmd = Command::new(bin);
        cmd.arg("lint")
            .arg("--reporter=json")
            .arg(format!("--stdin-file-path={}", shown(path)))
            .current_dir(root);
        out.push(Pass {
            cmd,
            input: Some(text.into_bytes()),
            rename: Some((*path).clone()),
        });
    }
    if !plain.is_empty() {
        let mut cmd = Command::new(bin);
        cmd.arg("lint")
            .arg("--reporter=json")
            .arg("--no-errors-on-unmatched")
            .args(&plain)
            .current_dir(root);
        out.push(Pass {
            cmd,
            input: None,
            rename: None,
        });
    }
    out
}

// One lint pass and the faults its report holds; the reason when the pass said nothing useful.
fn report(
    cmd: Command,
    input: Option<&[u8]>,
    root: &Path,
    stop: &Arc<Stop>,
    rename: Option<&Path>,
) -> Result<Vec<Fault>, String> {
    let Ok(ran) = gate::run(cmd, input, None, stop) else {
        return Err("biome did not run".to_string());
    };
    if ran.stopped {
        return Err("the turn was stopped".to_string());
    }
    let Ok(report) = serde_json::from_str::<serde_json::Value>(ran.out.trim()) else {
        // A lint that found nothing prints an empty report; one that refused says why on its stderr.
        return if ran.code == Some(0) {
            Ok(Vec::new())
        } else {
            Err(failure(&ran))
        };
    };
    let mut out = Vec::new();
    let found = report.get("diagnostics").and_then(|d| d.as_array());
    for one in found.into_iter().flatten() {
        if let Some(fault) = biome_fault(root, one, rename) {
            out.push(fault);
        }
    }
    Ok(out)
}

// What biome said when it left no report behind.
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
        Some(line) => format!("biome exited with {code}: {line}"),
        None => format!("biome exited with {code} without a word"),
    }
}

// One biome diagnostic as a fault: its file, its line, what it said and the rule it named. A report about a text handed over on stdin names that text's stand-in, so the file it belongs to is said here instead.
fn biome_fault(root: &Path, one: &serde_json::Value, rename: Option<&Path>) -> Option<Fault> {
    let message = one
        .get("message")
        .and_then(|m| m.as_str())?
        .trim()
        .to_string();
    let place = one.get("location")?;
    let file = place.get("path").and_then(|p| p.as_str())?;
    let line = place
        .get("start")
        .and_then(|s| s.get("line"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(1) as usize;
    let mut helps: Vec<String> = Vec::new();
    if let Some(category) = one.get("category").and_then(|c| c.as_str()) {
        helps.push(category.to_string());
    }
    let named = rename.unwrap_or_else(|| Path::new(file));
    Some(Fault {
        file: parts::rel(root, named),
        line,
        message,
        helps,
    })
}

// The name biome reads the kind from: `.htm` is not one it knows, so a page is handed over as `.html`.
pub(super) fn shown(path: &Path) -> String {
    if path.extension().and_then(|e| e.to_str()) == Some("htm") {
        return path.with_extension("html").display().to_string();
    }
    path.display().to_string()
}

// The biome binary: beside this executable first, then whatever PATH holds.
pub(super) fn program() -> Option<PathBuf> {
    static FOUND: LazyLock<Mutex<Option<Option<PathBuf>>>> = LazyLock::new(|| Mutex::new(None));
    if let Some(hit) = lock(&FOUND).as_ref() {
        return hit.clone();
    }
    let beside = gate::beside("biome");
    let found = beside.is_file().then_some(beside).or_else(|| {
        let dirs = std::env::var_os("PATH")?;
        std::env::split_paths(&dirs)
            .map(|dir| dir.join("biome"))
            .find(|candidate| candidate.is_file())
    });
    *lock(&FOUND) = Some(found.clone());
    found
}

// A lock that survives a panicking holder: the state behind it is a cache, never a promise.
fn lock<T>(held: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    held.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
