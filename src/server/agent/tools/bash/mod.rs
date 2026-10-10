//! bash: run one command, pass it through the guard, expand `#<ref>` into a temp path.

pub(super) mod guard;
mod shell;

use anyhow::{Result, bail};

use super::spec::{Args, Artifacts, Env, Param, Tool, ToolOutput, ToolProgress};
use crate::server::storage::parse_base36;

const COMMAND: usize = 0;

pub(super) const PARAMS: &[Param] = &[Param::req(
    "command",
    super::spec::Kind::Text,
    "One shell command.",
)];

pub(super) const TOOL: Tool = Tool {
    name: "bash",
    about: "Run one shell command (`bash -c` semantics; a single command, no newlines). Pipes, redirection, and\n\
            composition are fine. The working directory is the permitted zone: deleting or moving things inside it\n\
            is allowed, but rm/mv/find -delete that reaches outside (including ~ and system paths like /etc) is\n\
            refused outright. Catastrophic patterns are blocked the same way: `dd` to a block device, fork bombs,\n\
            reverse shells, `curl | sh`. A `#<ref>` token (e.g. `#3xl`) stands for a stored artifact — it turns\n\
            into a file holding that whole text, so `grep x #3xl` searches it; a token that names nothing stays\n\
            literal text.",
    params: PARAMS,
    run,
};

fn run(
    env: &mut Env,
    args: &Args,
    progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    let command = args.str(COMMAND)?;
    let cmd = command.trim();
    anyhow::ensure!(!cmd.is_empty(), "empty command");
    anyhow::ensure!(!cmd.contains('\n'), "one command at a time (no newlines)");

    // `#<ref>` virtual files: rewrite first, then guard — the rewritten paths are read-only temp
    // files, and the guard reads them as plain paths.
    let artifacts = env.inbox.artifacts.clone();
    let cmd = match &artifacts {
        Some(a) => rewrite_refs(cmd, a.as_ref())?,
        None => cmd.to_string(),
    };

    let cwd = env.workspace.cwd().to_path_buf();
    let zone = cwd.canonicalize().unwrap_or_else(|_| cwd.clone());
    let timeout = env.bash.timeout;
    let interrupt = env.bash.interrupt.clone();

    let result = match guard::classify(&cmd, &zone) {
        guard::Verdict::Block(hits) => {
            let ids: Vec<&str> = hits.iter().map(|h| h.rule_id).collect();
            bail!("denied: {}", ids.join("+"));
        }
        guard::Verdict::Warn(hits) => {
            // A warning hit: run anyway, tagging the output so the model sees what it did.
            let tags: Vec<&str> = hits.iter().map(|h| h.rule_id).collect();
            let text = shell::run(&cwd, &cmd, timeout, interrupt, progress)?;
            format!("[warn: {}]\n{text}", tags.join("+"))
        }
        guard::Verdict::Allow => shell::run(&cwd, &cmd, timeout, interrupt, progress)?,
    };

    // The oversized tail is the front door's business (file it, append the ref); the tool hands up text.
    Ok(ToolOutput::text(result))
}

/// Turn every standalone `#<ref>` token in `cmd` into a temp path holding that artifact. The
/// replacement is `cat <file>` (the artifact's **content**); unknown tokens stay literal.
fn rewrite_refs(cmd: &str, artifacts: &dyn Artifacts) -> Result<String> {
    let mut rewritten = String::with_capacity(cmd.len());
    let mut resolved: Vec<(String, std::path::PathBuf)> = Vec::new();
    let bytes: Vec<char> = cmd.chars().collect();
    let is_boundary = |c: char| {
        c.is_whitespace() || matches!(c, '|' | ';' | '&' | '<' | '>' | '"' | '\'' | '(' | ')')
    };
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == '#' && (i == 0 || is_boundary(bytes[i - 1])) {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_alphanumeric() {
                j += 1;
            }
            let standalone = j == bytes.len() || is_boundary(bytes[j]);
            if j > i + 1 && standalone {
                let word: String = bytes[i + 1..j]
                    .iter()
                    .collect::<String>()
                    .to_ascii_lowercase();
                let path = match resolved.iter().find(|(w, _)| *w == word) {
                    Some((_, p)) => Some(p.clone()),
                    None => match resolve(&word, artifacts) {
                        Some(p) => {
                            resolved.push((word.clone(), p.clone()));
                            Some(p)
                        }
                        None => None,
                    },
                };
                if let Some(path) = path {
                    rewritten.push_str(&format!("cat {}", path.display()));
                    i = j;
                    continue;
                }
            }
        }
        rewritten.push(bytes[i]);
        i += 1;
    }
    Ok(rewritten)
}

/// One ref → one temp path. `None` = the artifact could not be fetched (leave the token as is).
fn resolve(word: &str, artifacts: &dyn Artifacts) -> Option<std::path::PathBuf> {
    let id = parse_base36(word)?;
    match artifacts.path_for(id) {
        Ok(path) => Some(path),
        Err(e) => {
            crate::server::platform::log::warn(
                "bash",
                format!("the artifact reference #{word} did not resolve: {e:#}"),
            );
            None
        }
    }
}
