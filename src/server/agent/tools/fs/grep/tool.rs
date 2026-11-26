// The `grep` tool: hits inside the source trees a language claims, each with the piece around it.

use super::{render, scan};
use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::output::ToolOutput;
use crate::server::agent::tools::contract::view::CallRecord;
use crate::server::agent::tools::spec::{Args, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;
use anyhow::Result;

const QUERY: usize = 0;
const REGEX: usize = 1;
const SCOPE: usize = 2;
const LAVISH: usize = 3;

const PARAMS: &[Param] = &[
    Param::new(
        "query",
        Kind::Text,
        "Literal text, regex (`regex: true`), or relation query (`A::b`, `A.b`, `#id`, `.class`, `Class.method`, or symbol name).",
    ),
    Param::df(
        "regex",
        Kind::Flag,
        "Treat `query` as a regular expression.",
        "false",
    ),
    Param::new(
        "scope",
        Kind::TextList,
        "Optional file or directory paths within the workspace's source trees (defaults to all detected trees; invalid paths are skipped).",
    ),
    Param::df(
        "lavish",
        Kind::Flag,
        "Raise output detail by one tier to reduce folding.",
        "false",
    ),
];

pub(crate) const TOOL: Tool = Tool {
    name: "grep",
    about: "Part of the FS family (`read`, `grep`, `splice`/`edit`, `write`). Structural and text search across the workspace's source trees (`Rust`, `Python`, `Go`, `C/C++`, `HTML/CSS/JS`, `R`, `Jupyter Notebook`, `Markdown`, `JSON`, `TOML`, `YAML`, `XML`). Returns exact 1-based hit line numbers inside enclosing item hierarchies and authorizes displayed files for editing.",
    fields: "- Relation queries: Beyond literal/regex hits, `query` triggers cross-file symbol/declaration lookup for `Rust` and `C/C++` (`A::b` or bare name), `Python` and `Go` (`A.b` or bare name), `Web` (`#id` and `.class` across HTML/CSS/JS, `Class.method`, or bare script name), and `R` (bare function name).\n\
             - Output & tiers: Header reports hit/item/file counts, density tier (`1`-`3`), and active `scope` paths. If the footer reports `DOWNSHIFTED` (tier 2 or 3), pass `lavish: true` for denser output or narrow `scope`. Any omitted items are listed at the bottom.",
    params: PARAMS,
    run,
    summary,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let query = args.str(QUERY)?;
    anyhow::ensure!(!query.is_empty(), "`query` is empty");
    let regex = args.flag(REGEX)?;
    let lavish = args.flag(LAVISH)?;
    let zone = scan::zone(&env.workspace.cwd());
    let matcher = if regex {
        scan::Matcher::pattern(&query)?
    } else {
        scan::Matcher::text(&query)
    };
    let mut scope = args.str_list(SCOPE)?;
    let (dirs, picked, misses) = scan::roots(&zone, &mut scope);
    let files = scan::collect(&dirs, &picked, &matcher);
    let mode = if regex { "regex" } else { "literal" };
    let (text, shown) = render::answer(
        &query, mode, &files, &zone, env.grep, lavish, &misses, &scope,
    );
    render::stamp(&mut env.seen, &files, &shown);
    Ok(ToolOutput::text(text))
}

// One line for a history listing: the query.
fn summary(call: &CallRecord) -> String {
    summary::flat(&summary::field(call, "query").unwrap_or_default())
}
