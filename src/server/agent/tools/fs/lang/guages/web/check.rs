// The web check: biome lints the files a batch wrote, inside the page's own root.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

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
    // A written file biome's own configuration leaves out is no error of the batch: the pass runs to its end and the report says what it read.
    let mut cmd = Command::new(gate::beside("biome"));
    cmd.arg("lint")
        .arg("--reporter=json")
        .args(&files)
        .current_dir(root);
    let mut warns = match report(cmd, root, stop) {
        Ok(found) => found,
        Err(why) => return Check::Unchecked(why),
    };
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

// One lint pass and the faults its report holds; the reason when the pass said nothing useful.
fn report(cmd: Command, root: &Path, stop: &Arc<Stop>) -> Result<Vec<Fault>, String> {
    let Ok(ran) = gate::run(cmd, None, None, stop) else {
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
    // How many files the pass actually read: a file the configuration covers none of is a file this report cannot speak for.
    let counted: u64 = ["changed", "unchanged"]
        .iter()
        .filter_map(|key| report.get("summary")?.get(*key)?.as_u64())
        .sum();
    if counted == 0 {
        return Err(format!(
            "biome read none of the written files: its configuration in {} does not cover them",
            root.display()
        ));
    }
    let mut out = Vec::new();
    let found = report.get("diagnostics").and_then(|d| d.as_array());
    for one in found.into_iter().flatten() {
        if let Some(fault) = biome_fault(root, one) {
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

// One biome diagnostic as a fault: its file, its line, what it said and the rule it named.
fn biome_fault(root: &Path, one: &serde_json::Value) -> Option<Fault> {
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
    Some(Fault {
        file: parts::rel(root, Path::new(file)),
        line,
        message,
        helps,
    })
}
