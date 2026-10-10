//! search: search the web.

use anyhow::Result;
use serde_json::json;

use super::spec::{Args, Env, Kind, Param, Tool, ToolOutput, ToolProgress};

const QUERY: usize = 0;
const LIMIT: usize = 1;
const RECENCY: usize = 2;
const ENGINE: usize = 3;

pub(super) const PARAMS: &[Param] = &[
    Param::req(
        "query",
        Kind::Text,
        "Search terms; supports site: `phrase` -excluded filetype: inurl: intitle: before:/after:",
    ),
    Param::opt("limit", Kind::Number, "Result count. Default 8, max 20."),
    Param::opt(
        "recency",
        Kind::Choice(&["day", "week", "month", "year"]),
        "Only results published inside this window. Omit for no time filter.",
    ),
    Param::opt(
        "engine",
        Kind::Choice(&[
            "auto",
            "public",
            "bing",
            "duckduckgo",
            "google",
            "ecosia",
            "startpage",
            "mojeek",
        ]),
        "Which engine(s) to use. auto (default) tries them in turn and stops at the first that answers; public queries all of them in parallel and merges by agreement (slower, widest coverage); naming one engine pins it.",
    ),
];

pub(super) const TOOL: Tool = Tool {
    name: "search",
    about: "Web search; returns titles, links, snippets, and the engine that found each result.\n\
            Advanced syntax: site:github.com (limit to a domain), `exact phrase`, -excluded, filetype:,\n\
            inurl:, intitle:, before:/after: (YYYY-MM-DD). Directives an engine cannot honor are\n\
            enforced on the results afterwards, and a constraint that would empty the list is relaxed\n\
            with a `Note:` in the output. Use it for current facts, pinpointing documentation, and\n\
            finding source repositories; `engine` chooses breadth (`public` asks every engine at once\n\
            and ranks by agreement) or pins one engine.",
    params: PARAMS,
    run,
};

fn run(
    env: &mut Env,
    args: &Args,
    _progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    // Engine and recency are web's own enums: its serde reads them, so the names live in one place.
    let a: crate::web::SearchArgs = serde_json::from_value(json!({
        "intent": args.intent(),
        "query": args.str(QUERY)?,
        "limit": args.opt_int(LIMIT)?,
        "recency": args.opt_str(RECENCY)?,
        "engine": args.opt_str(ENGINE)?,
    }))?;
    let outcome = crate::web::search(&a, &env.browser)?;
    Ok(ToolOutput::text(crate::web::render(&outcome)))
}
