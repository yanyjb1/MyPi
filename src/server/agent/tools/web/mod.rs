//! The `web` tool: map a call's arguments into the domain's input, and hand the result back.

use anyhow::Result;

use super::contract::env::Env;
use super::contract::output::ToolOutput;
use super::contract::view::CallRecord;
use super::spec::{Args, Kind, Param, Tool, ToolProgress};
use super::summary;

const TYPE: usize = 0;
const URL: usize = 1;
const RAW: usize = 2;
const QUERY: usize = 3;
const PROVIDER: usize = 4;
const PAGES: usize = 5;
const SELECTOR: usize = 6;
const VALUE: usize = 7;
const PIXELS: usize = 8;
const FULL_PAGE: usize = 9;

/// Every operation; the schema and the dispatch read this one list.
const OPS: &[&str] = &[
    "read",
    "search",
    "open",
    "click",
    "fill",
    "select",
    "scroll",
    "eval",
    "screenshot",
];

const PARAMS: &[Param] = &[
    Param::new("type", Kind::Choice(OPS), "Which operation this is."),
    Param::new("url", Kind::Text, "The page to load."),
    Param::df(
        "raw",
        Kind::Flag,
        "Read the DOM itself instead of Markdown; it is written to a file and the answer is its path",
        "false",
    ),
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
    Param::new("selector", Kind::Text, "CSS selector."),
    Param::new(
        "value",
        Kind::Text,
        "Text for the field, or the JavaScript.",
    ),
    Param::df("pixels", Kind::Number, "How far to scroll down.", "600"),
    Param::df(
        "full_page",
        Kind::Flag,
        "Capture the whole page, not the viewport.",
        "false",
    ),
];

pub(crate) const TOOL: Tool = Tool {
    name: "web",
    about: "Read, search, and drive a real browser. `read` answers Markdown, or the DOM as a file \
            with `raw`; `search` runs one engine and merges pages 1..N; `open`/`click`/`fill`/\
            `select`/`scroll`/`eval` operate the work tab; `screenshot` writes a PNG. Files land \
            under the system temp directory.",
    input: "type is always needed. read: url, or nothing to read the work tab as it stands; raw \
            writes the DOM to a file and answers its path. open: url, or nothing for a blank tab. \
            search: query and provider, pages to merge more than the first. click: selector. \
            fill, select: selector and value. scroll: pixels. eval: value, the JavaScript to run. \
            screenshot: full_page for the whole page.",
    params: PARAMS,
    run,
    summary,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let kind = args.choice(TYPE)?;
    let pages = args.int(PAGES)?;
    anyhow::ensure!(
        (1..=u32::MAX as i64).contains(&pages),
        "`pages` must be at least 1"
    );
    let value = match kind.as_str() {
        "scroll" => Some(serde_json::Value::from(args.int(PIXELS)?)),
        _ => args.opt_str(VALUE)?.map(serde_json::Value::from),
    };
    let a = crate::web::WebArgs {
        op: kind,
        url: args.opt_str(URL)?,
        raw: args.flag(RAW)?,
        query: args.opt_str(QUERY)?,
        provider: args.opt_str(PROVIDER)?,
        pages: pages as u32,
        selector: args.opt_str(SELECTOR)?,
        value,
        full_page: args.flag(FULL_PAGE)?,
    };
    let text = crate::web::run(&a, env.session, &env.rules)?;
    Ok(ToolOutput::text(text))
}

/// One line for a history listing: the operation and the argument that locates it.
fn summary(call: &CallRecord) -> String {
    let op = summary::field(call, "type").unwrap_or_default();
    let at = match op.as_str() {
        "read" => summary::field(call, "url").unwrap_or_else(|| "page".to_string()),
        "open" => summary::field(call, "url").unwrap_or_default(),
        "search" => summary::field(call, "query").unwrap_or_default(),
        "click" | "fill" | "select" => summary::field(call, "selector").unwrap_or_default(),
        "eval" => summary::field(call, "value").unwrap_or_default(),
        _ => String::new(),
    };
    if at.trim().is_empty() {
        summary::clip(&op, 96)
    } else {
        summary::clip(&format!("{op} {at}"), 96)
    }
}
