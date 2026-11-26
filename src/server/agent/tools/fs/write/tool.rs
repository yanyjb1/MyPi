// The `write` tool: overwrite a whole file. The batch's audit type-checks it after the batch's last write; this call judges the text, writes it as it came, and hands the file's shell to the audit.
use std::path::Path;

use anyhow::{Context as _, Result};

use crate::server::agent::tools::contract::env::{Env, Shell, Slot};
use crate::server::agent::tools::contract::halt::stopped;
use crate::server::agent::tools::contract::output::ToolOutput;

use crate::server::agent::tools::fs::eol;
use crate::server::agent::tools::fs::fault;
use crate::server::agent::tools::fs::lang;
use crate::server::agent::tools::fs::lang::graph::parts;
use crate::server::agent::tools::fs::lang::verdict::Outcome;
use crate::server::agent::tools::spec::{Args, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;
use crate::server::agent::tools::workspace::write_atomic;

const PATH: usize = 0;
const CONTENT: usize = 1;
const UNFOLD: usize = 2;

// What `content` says instead of a file's text: the file is deleted.
const DELETE_FILE: &str = "__DELETE_FILE__";

// The name a commit message is written to, at the workspace root.
const COMMIT_MESSAGE: &str = ".commitmessage";

const PARAMS: &[Param] = &[
    Param::new(
        "path",
        Kind::Text,
        "Target file path (parent directory must exist).",
    ),
    Param::new(
        "content",
        Kind::Text,
        "Full file content to write verbatim.",
    ),
    Param::df(
        "unfold",
        Kind::Flag,
        "Expand all diagnostics and test output for this batch instead of folding.",
        "false",
    ),
];

pub(crate) const TOOL: Tool = Tool {
    name: "write",
    about: "Part of the FS family (`read`, `grep`, `splice`/`edit`, `write`). Creates a new file or **overwrites** an existing one verbatim (prefer `edit`/`splice` for targeted changes). Joins the batch's shared static check and targeted test audit reported at the end of the last file-modifying call in the message.",
    fields: "Parent directory must already exist. Preserves existing `CRLF`/`LF` line endings when overwriting. If the file's language has a parser, syntax errors are reported as `[unparsed]` with line details while still writing the file; wait for the batch's check output for the full compiler/linter verdict.\n\
             - No-shell sessions only (treated as literal text when a shell tool is available): writing `.commitmessage` at the workspace root formats all modified files with their language formatters (revoking read authorization for reformatted files) and creates a git commit; writing `content: \"__DELETE_FILE__\"` deletes an already-read, unmodified file.",
    params: PARAMS,
    run,
    summary: summary::path,
};

// Write the whole file, **overwriting** an existing one. The counterpart of `edit`: `edit` refuses ambiguous matches, `write` refuses nothing. The caller brings the full content.
fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let path = args.str(PATH)?;
    let content = args.str(CONTENT)?;
    let unfold = args.flag(UNFOLD)?;

    let p = env.workspace.resolve(&path, false)?;
    if let Some(parent) = p.parent() {
        anyhow::ensure!(
            parent.is_dir(),
            "parent directory does not exist: {} (create the directory first)",
            parent.display()
        );
    }
    // The commit message is a word to the session, not a file: it lands in the environment, and nothing touches the disk.
    if is_commit_message(env, &p) {
        return Ok(commit_message(env, &p, content.trim()));
    }
    // The delete sentinel is read before the text is judged: nothing lands in the file, and no parser has anything to say about a file about to go.
    // It needs the same hands as the commit message: with a shell the model deletes by itself, and this word is only a literal file's text.
    if env.bare_hands() && content.trim() == DELETE_FILE {
        return delete(env, &p);
    }
    let existed = p.exists();

    // The parser judges the text and leaves it whole: the caller's content is what lands.
    let parser = lang::format(&p);
    let mut unparsed = None;
    let stop = &env.stop;
    if let Some(format) = parser {
        match format(&content, &p, stop) {
            Outcome::Parsed => {}
            // A text that does not parse still lands as it came, and the answer names the line: the batch's check is the real verdict.
            Outcome::Unparsed(why) => unparsed = Some(why),
            // The parser was killed: the file is untouched, and the answer says why.
            Outcome::Stopped => return Ok(stopped(&p)),
        }
    }
    let written = content.clone();

    // The line ending is the file's own: a file that already ended its lines in CRLF keeps doing so, and a new one is LF.
    let ending = std::fs::read_to_string(&p)
        .map(|old| eol::Eol::of(&old))
        .unwrap_or(eol::Eol::Lf);
    let written = ending.write(&written);
    // What git knew of the file before this call touches it: the row a commit reads.
    let git = crate::git::facts(&p);
    env.workspace
        .with_write(&p, || write_atomic(&p, &written))?;
    env.seen.stamp(p.clone(), &written);
    env.ledger.note(p.clone(), git);
    let bytes = written.len();
    let mut text = format!(
        "{} {} ({bytes} bytes)",
        if existed { "overwrote" } else { "created" },
        p.display()
    );
    if let Some(why) = &unparsed {
        text.push('\n');
        text.push_str(&fault::unparsed(why));
    }

    // The compiler answers for the file as it now stands, after the batch's last write; the seam stands under this answer at its end. A file whose language names a checker, or tests alone, joins the batch's audit.
    let home = lang::checker(&p)
        .and_then(|checker| {
            let root = (checker.unit)(&p)?;
            let shown = (checker.shown)(&root, &p);
            Some((root, shown))
        })
        .or_else(|| {
            let support = lang::tests(&p)?;
            let root = (support.unit)(&p)?;
            let shown = parts::rel(&root, &p);
            Some((root, shown))
        });
    if let Some((root, shown)) = home {
        env.shells.push(Shell {
            call_id: env.current.clone(),
            mine: shown,
            root,
            file: p.clone(),
            unfold,
            seam: true,
            slots: vec![Slot {
                landing: None,
                insert_at: text.lines().count(),
            }],
        });
    }
    Ok(ToolOutput::text(text))
}

// The commit message is the workspace root's own name and nothing else, and only where no shell could have committed on its own.
fn is_commit_message(env: &Env, path: &Path) -> bool {
    env.bare_hands()
        && path.file_name().is_some_and(|name| name == COMMIT_MESSAGE)
        && path.parent() == Some(env.workspace.cwd().as_path())
}

// Hand the message to the audit, which commits once its check has passed. Two writes in one batch keep the last word, and nothing lands on disk.
fn commit_message(env: &mut Env, path: &Path, message: &str) -> ToolOutput {
    env.pending = Some(message.to_string());
    ToolOutput::text(format!(
        "commit queued for {}: {} (commits once this batch's check passes)",
        path.display(),
        message.lines().next().unwrap_or("")
    ))
}

// Delete one file the model asked to remove. The file must have been read, and must still hold what was read: a file the user changed since is theirs again, and the model reads it first.
fn delete(env: &mut Env, path: &Path) -> Result<ToolOutput> {
    let Some(license) = env
        .seen
        .files
        .get(path)
        .cloned()
        .filter(|held| held.approval)
    else {
        env.seen.revoke(path);
        anyhow::bail!(
            "`{}` is unauthorized: read it before deleting it",
            path.display()
        );
    };
    let text = std::fs::read_to_string(path).context("failed to read file")?;
    anyhow::ensure!(
        license.matches(&text),
        "`{}` changed on disk since it was read: read it again before deleting it",
        path.display()
    );
    let root = crate::git::root(&env.workspace.cwd());
    let tracked = root.as_ref().is_some_and(|root| {
        crate::git::relative(root, path).is_some_and(|name| crate::git::tracked(root, &name))
    });
    env.workspace.with_write(path, || {
        std::fs::remove_file(path).context("failed to delete it")
    })?;
    env.seen.files.remove(path);
    env.ledger.gone(path.to_path_buf(), tracked);
    Ok(ToolOutput::text(format!(
        "deleted {}{}",
        path.display(),
        if tracked {
            " (tracked: it joins this batch's commit)"
        } else {
            " (untracked: git holds no version of it)"
        }
    )))
}
