//! `/prepare` — the workspace facts in front of one message.

use std::path::{Path, PathBuf};

use super::meta::{Arg, CommandMeta};

use super::{Call, Done};

pub const META: CommandMeta = CommandMeta {
    name: "/prepare",
    aliases: &[],
    args: &[Arg::Text],
    detail: "cmd_prepare",
    run,
};

pub fn run(call: &Call<'_>) -> Done {
    let text = call.args.to_string();
    Done::Submit {
        prefix: payload(call.cwd, !text.is_empty()),
        text,
    }
}

/// The whole preparation: the directory, the terms, their related files and the manifest.
/// `body` asks for the line that introduces the user's own text; that text follows the
/// prefix byte for byte.
pub(crate) fn payload(cwd: &str, body: bool) -> Option<String> {
    let cwd = cwd.trim();
    if cwd.is_empty() {
        return None;
    }
    let cwd = Path::new(cwd);
    let mut out = format!("cwd: {}\n", cwd.display());
    let picked = terms(cwd);
    if let Some((name, text)) = &picked {
        section(&mut out, name, text);
    }
    if picked.as_ref().is_some_and(|(name, _)| *name == AI) {
        for (path, text) in related(cwd) {
            section(&mut out, &path, &text);
        }
    }
    if let Some(text) = manifest(cwd) {
        section(&mut out, "Cargo.toml", &text);
    }
    if body {
        out.push('\n');
        out.push_str("==> user\n");
    }
    Some(out)
}

/// The terms file, in priority order: the first one that exists.
fn terms(cwd: &Path) -> Option<(&'static str, String)> {
    for name in TERMS {
        if let Ok(text) = std::fs::read_to_string(cwd.join(name)) {
            return Some((name, text));
        }
    }
    None
}

/// The files an AI.md declares under a `prepare` fence, relative to the workspace root.
fn related(cwd: &Path) -> Vec<(String, String)> {
    let Ok(text) = std::fs::read_to_string(cwd.join(AI)) else {
        return Vec::new();
    };
    let root = canonical(cwd);
    let mut seen: Vec<PathBuf> = vec![canonical(&cwd.join(AI))];
    let mut out = Vec::new();
    for name in declared(&text) {
        let target = canonical(&cwd.join(&name));
        if !target.starts_with(&root) || !target.is_file() || seen.contains(&target) {
            continue;
        }
        let Ok(body) = std::fs::read_to_string(&target) else {
            continue;
        };
        seen.push(target);
        out.push((name, body));
    }
    out
}

/// Every path listed inside a top-level fenced block whose info string is `prepare`. A
/// `prepare` fence shown inside another block — documentation, an example — is not one.
fn declared(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut open: Option<(bool, usize)> = None;
    let mut block = String::new();
    for line in text.lines() {
        let head = line.trim_start();
        let ticks = head.bytes().take_while(|byte| *byte == b'`').count();
        match open {
            None => {
                if ticks >= 3 {
                    let info = head[ticks..].trim();
                    open = Some((info == "prepare", ticks));
                    block.clear();
                }
            }
            Some((prepare, width)) => {
                let closes = ticks >= width && head[ticks..].trim().is_empty();
                if closes {
                    if prepare && let Ok(paths) = serde_yaml::from_str::<Vec<String>>(&block) {
                        out.extend(paths.into_iter().map(|path| path.trim().to_string()));
                    }
                    open = None;
                } else if prepare {
                    block.push_str(line);
                    block.push('\n');
                }
            }
        }
    }
    out.retain(|path| !path.is_empty());
    out
}

/// The manifest's own declarations: the four sections, comments and blank lines as they stand.
fn manifest(cwd: &Path) -> Option<String> {
    let text = std::fs::read_to_string(cwd.join("Cargo.toml")).ok()?;
    let mut out = String::new();
    let mut keep = false;
    for line in text.lines() {
        let head = line.trim();
        if head.starts_with('[') {
            keep = SECTIONS.contains(&head);
        }
        if keep {
            out.push_str(line);
            out.push('\n');
        }
    }
    (!out.is_empty()).then_some(out)
}

/// One section: a blank line, the path it came from, and the body.
fn section(out: &mut String, header: &str, body: &str) {
    out.push('\n');
    out.push_str("==> ");
    out.push_str(header);
    out.push('\n');
    out.push_str(body);
    if !body.ends_with('\n') {
        out.push('\n');
    }
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

const TERMS: [&str; 3] = ["AGENTS.md", "CLAUDE.md", "AI.md"];
const AI: &str = "AI.md";
const SECTIONS: [&str; 4] = [
    "[package]",
    "[workspace]",
    "[dependencies]",
    "[dev-dependencies]",
];
