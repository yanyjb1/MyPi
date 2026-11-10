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
const PAGE: usize = 5;
const SELECTOR: usize = 6;
const KEY: usize = 7;
const VALUE: usize = 8;
const PIXELS: usize = 9;
const FULL_PAGE: usize = 10;

/// Every operation; the schema and the dispatch read this one list.
const OPS: &[&str] = &[
    "read",
    "search",
    "open",
    "click",
    "fill",
    "press",
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
        "Return DOM HTML instead of Markdown.",
        "false",
    ),
    Param::new(
        "query",
        Kind::Text,
        "The search words, passed through verbatim.",
    ),
    Param::new(
        "provider",
        Kind::Choice(&["google", "bing", "baidu"]),
        "The one engine this query runs against.",
    ),
    Param::df(
        "page",
        Kind::Number,
        "The last result page to fetch; pages 1..N are merged.",
        "1",
    ),
    Param::new("selector", Kind::Text, "CSS selector."),
    Param::new("key", Kind::Text, "Key name, e.g. `Enter`."),
    Param::new("value", Kind::Text, "The text to put into the field."),
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
    about: "Read, search, and drive a real browser. `read` returns Markdown or DOM HTML; `search` \
            runs one provider and merges pages 1..N; `open`/`click`/`fill`/`press`/`select`/\
            `scroll`/`eval` operate the work tab; `screenshot` writes a PNG under the system temp \
            directory.",
    input: "type is always needed. read: url, or nothing to read the work tab as it stands. \
            open: url, or nothing for a blank tab. search: query and provider. click: selector. \
            fill: selector and value. press: key. select: selector and value. scroll: pixels. \
            eval: value, the JavaScript to run. screenshot: nothing else.",
    params: PARAMS,
    run,
    summary,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let kind = args.choice(TYPE)?;
    let page = args.int(PAGE)?;
    let page = u32::try_from(page).map_err(|_| anyhow::anyhow!("`page` must be >= 1"))?;
    let selector = match kind.as_str() {
        "press" => args.opt_str(KEY)?,
        _ => args.opt_str(SELECTOR)?,
    };
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
        page,
        selector,
        value,
        full_page: args.flag(FULL_PAGE)?,
    };
    let text = crate::web::run(&a, env.session)?;
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
        "press" => summary::field(call, "key").unwrap_or_default(),
        "eval" => summary::field(call, "value").unwrap_or_default(),
        _ => String::new(),
    };
    if at.trim().is_empty() {
        summary::clip(&op, 96)
    } else {
        summary::clip(&format!("{op} {at}"), 96)
    }
}
