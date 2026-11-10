//! The one way raw command text becomes a spawnable line: aliases, stored texts, then the guard.

use anyhow::{Result, bail};

use super::{guard, segment};
use crate::server::agent::tools::contract::env::{Aliases, Env};
use crate::server::agent::tools::contract::view::Export;
use crate::server::platform::log;
use crate::server::platform::shell::Shell;

/// A command that passed the gate, plus the rule ids the guard warned about.
pub(crate) struct Admitted {
    pub cmd: String,
    pub warn: Option<String>,
}

/// Alias expansion, `#<ref>` rewriting, then the guard on the final text.
pub(crate) fn admit(env: &Env, raw: &str) -> Result<Admitted> {
    let shell = &env.bash.shell;
    let cmd = expand_alias(raw, &env.aliases);
    let cmd = rewrite_refs(&cmd, env.export.as_ref(), shell)?;
    if !shell.guarded() {
        return Ok(Admitted { cmd, warn: None });
    }
    let cwd = env.workspace.cwd().to_path_buf();
    let zone = cwd.canonicalize().unwrap_or_else(|_| cwd.clone());
    match guard::classify(&cmd, &zone) {
        guard::Verdict::Block(hits) => {
            let ids: Vec<&str> = hits.iter().map(|h| h.rule_id).collect();
            bail!("denied: {}", ids.join("+"))
        }
        guard::Verdict::Warn(hits) => {
            let ids: Vec<&str> = hits.iter().map(|h| h.rule_id).collect();
            Ok(Admitted {
                cmd,
                warn: Some(ids.join("+")),
            })
        }
        guard::Verdict::Allow => Ok(Admitted { cmd, warn: None }),
    }
}

/// Replace the head word of every command segment; one layer, quoted words kept.
fn expand_alias(cmd: &str, aliases: &Aliases) -> String {
    let mut out = String::with_capacity(cmd.len());
    let mut copied = 0;
    for (start, end) in segment::heads(cmd) {
        let Some(expansion) = aliases.get(&cmd[start..end]) else {
            continue;
        };
        out.push_str(&cmd[copied..start]);
        out.push_str(expansion);
        copied = end;
    }
    out.push_str(&cmd[copied..]);
    out
}

/// Turn every standalone `#<ref>` token in `cmd` into the quoted path of that stored text.
fn rewrite_refs(cmd: &str, export: &dyn Export, shell: &Shell) -> Result<String> {
    if !cmd.contains('#') {
        return Ok(cmd.to_string());
    }
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
                    .map(|(_, token)| token.clone());
                let token = match cached {
                    Some(token) => Some(token),
                    None => resolve(&word, export).map(|path| {
                        let token = shell.path_token(&path);
                        resolved.push((word.clone(), token.clone()));
                        token
                    }),
                };
                if let Some(token) = token {
                    rewritten.push_str(&token);
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

/// One ref → the path of its stored text. `None` = the ref names nothing (leave the token as is).
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
