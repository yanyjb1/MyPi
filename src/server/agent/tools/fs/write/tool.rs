// The `write` tool: overwrite a whole file. The batch's audit type-checks it after the batch's last write; this call only formats, writes, and hands the file's shell to the audit.
use std::fmt::Write as _;

use anyhow::Result;

use crate::server::agent::tools::contract::env::{Env, Shell, Slot};
use crate::server::agent::tools::contract::halt::stopped;
use crate::server::agent::tools::contract::output::ToolOutput;

use crate::server::agent::tools::fs::eol;
use crate::server::agent::tools::fs::fault;
use crate::server::agent::tools::fs::lang;
use crate::server::agent::tools::fs::lang::verdict::{Draft, Outcome};
use crate::server::agent::tools::spec::{Args, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;
use crate::server::agent::tools::workspace::write_atomic;

const PATH: usize = 0;
const CONTENT: usize = 1;
const UNFOLD: usize = 2;

const PARAMS: &[Param] = &[
    Param::new("path", Kind::Text, "File path."),
    Param::new(
        "content",
        Kind::Text,
        "The file's full content, written verbatim.",
    ),
    Param::df(
        "unfold",
        Kind::Flag,
        "List every fault of the batch's shared check instead of folding it.",
        "false",
    ),
];

pub(crate) const TOOL: Tool = Tool {
    name: "write",
    about: "Write the whole file, **overwriting** any existing one. A small change → `edit`. \
            Type-checked with the batch, reported on the last write.",
    fields: "both fields are always needed: the path, and the whole content. The parent folder \
             must already exist.\n\
             `unfold` lifts the shared check's fold for every written file of the batch.",
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
    let existed = p.exists();

    // The formatter runs first: the caller's content is what it meant, not what it lays out.
    let formatter = lang::format(&p);
    let mut unparsed = None;
    let mut undecided = None;
    let stop = &env.stop;
    let written = if let Some(format) = formatter {
        let draft = Draft {
            text: &content,
            runs: &[],
        };
        match format(&draft, &p, stop) {
            Outcome::Formatted { text, .. } => text,
            // A text the formatter cannot parse is written as it stands, and said so.
            Outcome::Unparsed(why) => {
                unparsed = Some(why);
                content.clone()
            }
            // The formatter decided nothing about the text: it is written as it stands, and the batch's check is left to say what it makes of it.
            Outcome::Undecided(why) => {
                undecided = Some(why);
                content.clone()
            }
            // The formatter was killed: the file is untouched, and the answer says why.
            Outcome::Stopped => return Ok(stopped(&p)),
        }
    } else {
        content.clone()
    };

    // The line ending is the file's own: a file that already ended its lines in CRLF keeps doing so, and a new one is LF.
    let ending = std::fs::read_to_string(&p)
        .map(|old| eol::Eol::of(&old))
        .unwrap_or(eol::Eol::Lf);
    let written = ending.write(&written);
    env.workspace
        .with_write(&p, || write_atomic(&p, &written))?;
    env.seen.stamp(p.clone(), &written);
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
    if let Some(why) = &undecided {
        text.push('\n');
        let _ = write!(text, "[i] not formatted, written as it stands: {why}");
    }

    // The compiler answers for the file as it now stands, after the batch's last write; the seam stands under this answer at its end. A file whose language names a checker joins the batch's check.
    if let Some(checker) = lang::checker(&p)
        && let Some(root) = (checker.unit)(&p)
    {
        env.shells.push(Shell {
            call_id: env.current.clone(),
            mine: (checker.shown)(&root, &p),
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
