// The commit a batch carries: its files set in their own language's style, its paths compared with what the user's git already held, and the survivors handed to git by path. Runs once, after the batch's last write.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::eol;
use super::lang;
use crate::server::agent::tools::contract::env::Env;
use crate::server::platform::stop::Stop;

// What one commit has to say: every line the committing answer prints, in order.
pub(crate) fn commit(env: &mut Env) -> Vec<String> {
    let Some(message) = env.pending.take() else {
        return Vec::new();
    };
    let Some(root) = crate::git::root(&env.workspace.cwd()) else {
        env.ledger.files.clear();
        return vec!["nothing committed: this directory is not in a git repository".to_string()];
    };
    let stop = env.stop.clone();
    let mut plan = set(env, &root, &stop);
    if plan.paths.is_empty() {
        // A refusal keeps the ledger: the model clears what stopped it, writes `.commitmessage` again, and the same files are named.
        if !plan.refused {
            env.ledger.files.clear();
        }
        return plan.lines;
    }
    // A file git never held must enter the index before a commit can name it; only these paths are added, so the user's own staged work stays theirs.
    let fresh: Vec<String> = plan
        .paths
        .iter()
        .filter(|name| !crate::git::tracked(&root, name))
        .cloned()
        .collect();
    let result = crate::git::add(&root, &fresh)
        .and_then(|()| crate::git::record(&root, &message, &plan.paths));
    match result {
        Ok(()) => plan.lines.push(format!(
            "committed {}: {}",
            plural(plan.paths.len()),
            subject(&message)
        )),
        // A refused commit keeps the ledger: the model fixes whatever git objected to, writes `.commitmessage` again, and the same files are named.
        Err(why) => {
            plan.lines.push(format!("commit refused: {why}"));
            return plan.lines;
        }
    }
    // A file the layout rewrote on disk no longer holds what the model read: its licence goes with the old text.
    for path in &plan.laid {
        env.seen.revoke(path);
    }
    env.ledger.files.clear();
    plan.lines
}

// What a commit hands git, and what it says about the rest.
struct Plan {
    // The files git may be told about, named against the repository root.
    paths: Vec<String>,
    // The files whose text a layout rewrote on disk.
    laid: Vec<PathBuf>,
    // What the answer says besides the commit itself.
    lines: Vec<String>,

    // The commit stopped for a reason the model can clear: the ledger stays, so a second `.commitmessage` names the same files.
    refused: bool,
}

// Lay out, compare, and sort the session's files into what a commit may name and what it must say instead.
fn set(env: &mut Env, root: &Path, stop: &Arc<Stop>) -> Plan {
    let mut plan = Plan {
        paths: Vec::new(),
        laid: Vec::new(),
        lines: Vec::new(),
        refused: false,
    };
    let mut collided: Vec<String> = Vec::new();
    let mut ignored: Vec<String> = Vec::new();
    for file in env.ledger.files.clone() {
        let Some(name) = crate::git::relative(root, &file.path) else {
            plan.lines.push(format!(
                "written outside the repository, nothing for git to record: {}",
                file.path.display()
            ));
            continue;
        };
        if file.deleted {
            if file.tracked {
                plan.paths.push(name);
            } else {
                plan.lines.push(format!(
                    "deleted an untracked file, nothing for git to record: {name}"
                ));
            }
            continue;
        }
        // The layout runs before the check: what lands is what the check then reads, line numbers and all.
        if let Some(text) = lay_out(&file.path, stop) {
            match write(&file.path, &text) {
                Ok(()) => plan.laid.push(file.path.clone()),
                Err(e) => {
                    plan.lines
                        .push(format!("{name}: the layout could not be written: {e}"));
                    continue;
                }
            }
        }
        // The user's own uncommitted work on this file would ride under the model's message, so the commit stops instead of taking it.
        if file.dirty {
            collided.push(name);
            continue;
        }
        if crate::git::ignored(root, &name) {
            ignored.push(name);
            continue;
        }
        plan.paths.push(name);
    }
    if !collided.is_empty() {
        plan.lines.push(format!(
            "commit refused: these files held your uncommitted changes before this session touched them: {}. Commit or stash them, then write `.commitmessage` again.",
            collided.join(", ")
        ));
        plan.paths.clear();
        plan.refused = true;
        return plan;
    }
    if !ignored.is_empty() {
        plan.lines.push(format!(
            "not committed, a rule in .gitignore holds them: {}",
            ignored.join(", ")
        ));
    }
    if plan.paths.is_empty() {
        plan.lines
            .push("nothing to commit: no tracked file of this session changed".to_string());
    }
    plan
}

// One file's text as its own formatter sets it. `None` = no formatter, a refusal, a stop, or a text that already reads that way.
fn lay_out(path: &Path, stop: &Arc<Stop>) -> Option<String> {
    let layout = lang::layout(path)?;
    let current = std::fs::read_to_string(path).ok()?;
    let text = layout(&current, path, stop)?;
    (text != current).then_some(text)
}

// A laid-out text back on disk, in the line ending the file already used.
fn write(path: &Path, text: &str) -> std::io::Result<()> {
    std::fs::write(path, eol::Eol::of(text).write(text))
}

// The count and its word: `1 file`, `3 files`.
fn plural(count: usize) -> String {
    if count == 1 {
        "1 file".to_string()
    } else {
        format!("{count} files")
    }
}

// The message's first line, cut short: what a one-line receipt says the commit was about.
fn subject(message: &str) -> String {
    let line = message.lines().next().unwrap_or("").trim();
    if line.is_empty() {
        "no message".to_string()
    } else {
        line.chars().take(60).collect()
    }
}
