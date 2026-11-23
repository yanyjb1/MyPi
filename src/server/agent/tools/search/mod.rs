// The `search` tool: one engine, one query, pages merged.

use anyhow::Result;

use super::contract::env::Env;
use super::contract::output::ToolOutput;
use super::contract::view::CallRecord;
use super::spec::{Args, Kind, Param, Tool, ToolProgress};
use super::summary;

const QUERY: usize = 0;
const PROVIDER: usize = 1;
const PAGES: usize = 2;

const PARAMS: &[Param] = &[
    Param::new(
        "query",
        Kind::Text,
        "The search words, passed through verbatim.",
    ),
    Param::new(
        "provider",
        Kind::Choice(crate::web::ENGINES),
        "The one engine this query runs against.",
    ),
    Param::df(
        "pages",
        Kind::Number,
        "How many result pages to merge, starting at 1",
        "1",
    ),
];

pub(crate) const TOOL: Tool = Tool {
    name: "search",
    about: "Search the web through one engine and merge pages 1..N. The pages themselves are \
            `web read`.",
    fields: "query and provider are always needed; pages merges more than the first.",
    params: PARAMS,
    run,
    summary,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let query = args.str(QUERY)?;
    let provider = args.choice(PROVIDER)?;
    let pages = args.int(PAGES)?;
    anyhow::ensure!(
        (1..=u32::MAX as i64).contains(&pages),
        "`pages` must be at least 1"
    );
    let a = crate::web::WebArgs {
        op: "search".to_string(),
        url: None,
        raw: false,
        query: Some(query),
        provider: Some(provider),
        pages: pages as u32,
        selector: None,
        value: None,
        full_page: false,
    };
    let text = crate::web::run(&a, env.session, &env.rules, &env.stop)?;
    Ok(ToolOutput::text(text))
}

// One line for a history listing: the query.
fn summary(call: &CallRecord) -> String {
    summary::flat(&summary::field(call, "query").unwrap_or_default())
}
