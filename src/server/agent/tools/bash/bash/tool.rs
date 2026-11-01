//! The `bash` tool: one command through the guard, `#<ref>` expanded into the shell's read of it.

use anyhow::{Result, bail};

use super::{guard, shell};
use crate::server::agent::tools::contract::env::{CallRecord, Env};
use crate::server::agent::tools::contract::output::ToolOutput;
use crate::server::agent::tools::contract::view::Export;
use crate::server::agent::tools::summary;
use crate::server::agent::tools::{Args, Kind, Param, Tool, ToolProgress};
use crate::server::platform::log;
use crate::server::platform::shell::Shell;

const COMMAND: usize = 0;

const PARAMS: &[Param] = &[Param::req("command", Kind::Text, "One shell command.")];

pub(crate) const TOOL: Tool = Tool {
    name: "bash",
    about: "Run one shell command in the session's shell (`bash` or `zsh` on unix, `pwsh` on windows; \
            a single command, no newlines). Pipes, redirection, and composition are fine. The working\n\
            directory is the permitted zone: deleting or moving things inside it is allowed, but rm/mv/find -delete\n\
            that reaches outside (including ~ and system paths like /etc) is refused outright. Catastrophic\n\
            patterns are blocked the same way: `dd` to a block device, fork bombs, reverse shells, `curl | sh`.\n\
            A `#<ref>` token (e.g. `#3xl`) stands for a stored artifact — it turns into a file holding that\n\
            whole text, so `grep x #3xl` searches it; a token that names nothing stays literal text.",
    params: PARAMS,
    run,
    summary: Some(summary),
};

fn run(env: &mut Env, args: &Args, progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let command = args.str(COMMAND)?;
    let cmd = command.trim();
    anyhow::ensure!(!cmd.is_empty(), "empty command");
    anyhow::ensure!(!cmd.contains('\n'), "one command at a time (no newlines)");

    // Alias expansion, then `#<ref>` rewriting, then the guard — the guard sees the final command.
    let backend = env.bash.shell.clone();
    let cmd = expand_alias(cmd, &env.aliases);
    let cmd = rewrite_refs(&cmd, env.export.as_ref(), &backend)?;

    let cwd = env.workspace.cwd().to_path_buf();
    let timeout = env.bash.timeout;
    let interrupt = env.bash.interrupt.clone();

    if !backend.guarded() {
        return Ok(ToolOutput::text(shell::run(
            &backend, &cwd, &cmd, timeout, interrupt, progress,
        )?));
    }
    let zone = cwd.canonicalize().unwrap_or_else(|_| cwd.clone());
    let result = match guard::classify(&cmd, &zone) {
        guard::Verdict::Block(hits) => {
            let ids: Vec<&str> = hits.iter().map(|h| h.rule_id).collect();
            bail!("denied: {}", ids.join("+"));
        }
        guard::Verdict::Warn(hits) => {
            // A warning hit: run anyway, tagging the output so the model sees what it did.
            let tags: Vec<&str> = hits.iter().map(|h| h.rule_id).collect();
            let text = shell::run(&backend, &cwd, &cmd, timeout, interrupt, progress)?;
            format!("[warn: {}]\n{text}", tags.join("+"))
        }
        guard::Verdict::Allow => shell::run(&backend, &cwd, &cmd, timeout, interrupt, progress)?,
    };

    // The oversized tail is the front door's business (file it, append the ref); the tool hands up text.
    Ok(ToolOutput::text(result))
}

/// One line for a history listing: the command, flattened, and whether it failed.
fn summary(call: &CallRecord) -> String {
    let command = summary::field(call, "command").unwrap_or_else(|| call.args.clone());
    summary::clip(&command, 96)
}

/// Expand only the first token, only one layer; arguments keep their original separator.
pub(crate) fn expand_alias(
    cmd: &str,
    aliases: &std::collections::BTreeMap<String, String>,
) -> String {
    let Some((head, _)) = cmd.split_once(char::is_whitespace) else {
        return aliases.get(cmd).cloned().unwrap_or_else(|| cmd.to_string());
    };
    let Some(expansion) = aliases.get(head) else {
        return cmd.to_string();
    };
    format!("{expansion}{}", &cmd[head.len()..])
}

/// Turn every standalone `#<ref>` token in `cmd` into the shell's own read of that stored text.
pub(crate) fn rewrite_refs(cmd: &str, export: &dyn Export, shell: &Shell) -> Result<String> {
    let mut rewritten = String::with_capacity(cmd.len());
    let mut resolved: Vec<(String, String)> = Vec::new();
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
                let cached = resolved
                    .iter()
                    .find(|(w, _)| *w == word)
                    .map(|(_, read)| read.clone());
                let read = match cached {
                    Some(read) => Some(read),
                    None => resolve(&word, export).map(|path| {
                        let read = shell.read_file(&path);
                        resolved.push((word.clone(), read.clone()));
                        read
                    }),
                };
                if let Some(read) = read {
                    rewritten.push_str(&read);
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

/// One ref → the token that prints it. `None` = the ref names nothing (leave the token as is).
fn resolve(word: &str, export: &dyn Export) -> Option<std::path::PathBuf> {
    match export.path(word) {
        Ok(path) => Some(path),
        Err(e) => {
            log::warn(
                "bash",
                format!("the artifact reference #{word} did not resolve: {e:#}"),
            );
            None
        }
    }
}
