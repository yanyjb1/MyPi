// The Rust check: cargo types one crate whole, and its JSON becomes the diagnostics a report draws.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, LazyLock};

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

// The crate this file belongs to: the nearest directory above it that holds a manifest.
pub(super) fn unit(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|dir| dir.join("Cargo.toml").is_file())
        .map(Path::to_path_buf)
}

// The crate's own name, from the manifest's `[package] name`; the folder when unnamed.
pub(super) fn title(root: &Path) -> String {
    let named = std::fs::read_to_string(root.join("Cargo.toml"))
        .ok()
        .and_then(|text| package_name(&text));
    named.unwrap_or_else(|| {
        root.file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_default()
    })
}

// Type-check one crate to completion, in the compiler's own order. No deadline: a batch checks once, and only the turn's stop ends it. Cargo has no per-file mode, so the files this batch wrote only say which unit to check, never which part of one.
fn run(root: &Path, _touched: &[PathBuf], stop: &Arc<Stop>) -> Check {
    let mut cmd = Command::new(cargo());
    cmd.args(["check", "--message-format=json", "--quiet"])
        .current_dir(root);
    let Ok(ran) = gate::run(cmd, None, None, stop) else {
        return Check::Unchecked("cargo did not run".to_string());
    };
    if ran.stopped {
        return Check::Unchecked("the turn was stopped".to_string());
    }
    let (mut faults, mut warns) = diagnostics(root, &ran.out);
    faults.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.message == b.message);
    warns.dedup_by(|a, b| a.file == b.file && a.line == b.line && a.message == b.message);
    if faults.is_empty() && warns.is_empty() {
        // A cargo that failed before it compiled leaves no diagnostic behind: its exit code is the only evidence the crate was never checked.
        return if ran.code == Some(0) {
            Check::Clean
        } else {
            Check::Unchecked(failure(&ran))
        };
    }
    Check::Faults { faults, warns }
}

// What a cargo that left no diagnostic behind said about itself.
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
        Some(line) => format!("cargo exited with {code}: {line}"),
        None => format!("cargo exited with {code} without a word"),
    }
}

// The errors and the warnings one `cargo check` reported, whatever file they sit in.
pub(super) fn diagnostics(root: &Path, out: &str) -> (Vec<Fault>, Vec<Fault>) {
    let mut errors: Vec<Fault> = Vec::new();
    let mut warns: Vec<Fault> = Vec::new();
    for line in out.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("reason").and_then(|r| r.as_str()) != Some("compiler-message") {
            continue;
        }
        let Some(message) = value.get("message") else {
            continue;
        };
        let list = match message.get("level").and_then(|l| l.as_str()) {
            Some("error") => &mut errors,
            Some("warning") => &mut warns,
            _ => continue,
        };
        let Some(spans) = message.get("spans").and_then(|s| s.as_array()) else {
            continue;
        };
        let span = spans
            .iter()
            .find(|span| span.get("is_primary").and_then(serde_json::Value::as_bool) == Some(true))
            .or_else(|| spans.first());
        let Some(span) = span else {
            continue;
        };
        let Some(file) = span.get("file_name").and_then(|f| f.as_str()) else {
            continue;
        };
        let line = span
            .get("line_start")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(1) as usize;
        let text = message
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("error")
            .trim()
            .to_string();
        let mut helps: Vec<String> = Vec::new();
        for child in message
            .get("children")
            .and_then(|c| c.as_array())
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            if child.get("level").and_then(|l| l.as_str()) != Some("help") {
                continue;
            }
            let Some(said) = child.get("message").and_then(|m| m.as_str()) else {
                continue;
            };
            // A suggestion names its replacement in a span, not in its own sentence.
            let named = child
                .get("spans")
                .and_then(|s| s.as_array())
                .and_then(|spans| spans.first())
                .and_then(|span| span.get("suggested_replacement"))
                .and_then(|name| name.as_str())
                .filter(|name| !name.contains('\n'));
            let said = match named {
                Some(name) => format!("{}: {name}", said.trim()),
                None => said.trim().to_string(),
            };
            if !said.is_empty() && !helps.contains(&said) {
                helps.push(said);
            }
        }
        helps.truncate(2);
        let file = Path::new(file);
        let named = if file.is_absolute() {
            file.to_path_buf()
        } else {
            root.join(file)
        };
        list.push(Fault {
            file: parts::rel(root, &named),
            line,
            message: text,
            helps,
        });
    }
    (errors, warns)
}

// The first `name = "…"` under `[package]`.
fn package_name(manifest: &str) -> Option<String> {
    let mut inside = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line == "[package]";
            continue;
        }
        if !inside || line.starts_with('#') {
            continue;
        }
        let Some(rest) = line.strip_prefix("name") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        let value = rest.trim().trim_matches('"').trim_matches('\'');
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

// The `cargo` to run: the one beside this executable first, then whatever PATH holds.
pub(super) fn cargo() -> PathBuf {
    static FOUND: LazyLock<PathBuf> = LazyLock::new(|| gate::beside("cargo"));
    FOUND.clone()
}
