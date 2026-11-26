// The `edit` tool: replace one unique run of text.

use anyhow::{Context as _, Result, anyhow, bail};
use serde_json::Value;

use crate::server::agent::tools::contract::env::{Env, Shell, Slot};
use crate::server::agent::tools::contract::output::ToolOutput;

use crate::server::agent::tools::fs::change;
use crate::server::agent::tools::fs::eol;
use crate::server::agent::tools::fs::lang;
use crate::server::agent::tools::spec::{Args, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;
use crate::server::agent::tools::workspace::write_atomic;

const PATH: usize = 0;
const OLD: usize = 1;
const NEW: usize = 2;
const UNFOLD: usize = 3;

const PARAMS: &[Param] = &[
    Param::new(
        "path",
        Kind::Text,
        "Target file path (must already be read in this conversation).",
    ),
    Param::new(
        "old",
        Kind::Text,
        "Exact text to replace; must appear character-for-character once in the file.",
    ),
    Param::new(
        "new",
        Kind::Text,
        "Replacement text (`\"\"` deletes `old`).",
    ),
    Param::df(
        "unfold",
        Kind::Flag,
        "Expand all diagnostics for this batch instead of folding.",
        "false",
    ),
];

pub(crate) const TOOL: Tool = Tool {
    name: "edit",
    about: "Part of the FS family (`read`, `grep`, `edit`, `write`). Replaces one unique exact text span in a previously read file (use `write` for new files or full rewrites). Batch static checks (`Rust`, `Python`, `Go`, `HTML/CSS/JS`, `C/C++`, `Notebook`) run once per message and appear at the end of the last `edit`/`write` call.",
    fields: "Fails if the file has not been read in this conversation, or if `old` matches 0 times or more than once (include more surrounding lines to make `old` unique). Preserves the file's existing line endings (`LF` / `CRLF`).",
    params: PARAMS,
    run,
    summary: summary::path,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let path = args.str(PATH)?;
    let p = env.workspace.resolve(&path, true)?;
    let old = args.str(OLD)?;
    let new = args.str(NEW)?;
    let unfold = args.flag(UNFOLD)?;

    // A write needs a read: the conversation holds the licence, and a refused one is taken away.
    let licensed = env
        .seen
        .files
        .get(&p)
        .is_some_and(|license| license.approval);
    if !licensed {
        env.seen.revoke(&p);
        bail!(
            "`{}` is unauthorized: it has not been read in this conversation",
            p.display()
        );
    }

    let edited = env.workspace.with_write(&p, || {
        // Blocking: the whole file is read and written inside the per-path lock, so every other writer waits.
        let raw = std::fs::read_to_string(&p).context("failed to read file")?;
        // The file's own line ending, kept: the tools match and splice on plain lines and write the ending back.
        let ending = eol::Eol::of(&raw);
        let text = ending.plain(&raw);
        let table = lang::table(&p, &text);
        let mut edited = exact_text(&text, &table, &old, &new)?;
        edited.text = ending.write(&edited.text);
        write_atomic(&p, &edited.text)?;
        Ok(edited)
    })?;
    env.seen.stamp(p.clone(), &edited.text);
    // What git knew of the file before this call touched it: the ledger keeps it for a commit that follows.
    env.ledger.note(p.clone(), crate::git::facts(&p));
    let last = (edited.at + edited.lines).saturating_sub(1);
    let text = format!("{}: {}", edited.receipt, p.display());

    // The compiler answers for the file after the batch's last write; its faults hang under the changed lines.
    if let Some(checker) = lang::checker(&p)
        && let Some(root) = (checker.unit)(&p)
    {
        env.shells.push(Shell {
            call_id: env.current.clone(),
            mine: (checker.shown)(&root, &p),
            root,
            file: p.clone(),
            unfold,
            seam: false,
            slots: vec![Slot {
                landing: Some((edited.at, last)),
                insert_at: text.lines().count(),
            }],
        });
    }
    Ok(ToolOutput::with_memo(text, edited.memo))
}

// One exact replacement: the new text, the item around it, the receipt, and where it landed.
pub(crate) struct Edited {
    pub(crate) text: String,
    pub(crate) memo: Value,
    pub(crate) receipt: String,
    pub(crate) at: usize,
    pub(crate) lines: usize,
}

// Replace one unique run
pub(crate) fn exact_text(text: &str, table: &lang::Table, old: &str, new: &str) -> Result<Edited> {
    anyhow::ensure!(!old.is_empty(), "`old` must not be empty");

    match text.matches(old).count() {
        0 => Err(anyhow!("`old` not found in the file")),
        1 => {
            let pos = text.find(old).expect("count == 1 checked above");
            let at = change::line_of(text, pos);
            let old_lines = change::lines_of(old).len();
            let new_lines = change::lines_of(new).len();
            let total = text.lines().count() + new_lines - old_lines;
            Ok(Edited {
                text: text.replacen(old, new, 1),
                memo: change::context(table, text, at, old_lines),
                receipt: change::receipt(at, old_lines, new_lines, total),
                at,
                lines: new_lines,
            })
        }
        n => Err(anyhow!(
            "`old` appears {n} times; make it longer with surrounding context, \
             or use `splice` to replace by line number"
        )),
    }
}
