use anyhow::{Result, anyhow};

use super::spec::{Args, Env, Kind, Param, Tool, ToolOutput, ToolProgress};

const OP: usize = 0;
const URL: usize = 1;
const RAW: usize = 2;
const QUERY: usize = 3;
const PROVIDER: usize = 4;
const PAGE: usize = 5;
const SELECTOR: usize = 6;
const VALUE: usize = 7;
const FILTER: usize = 8;
const MAX: usize = 9;
const FULL_PAGE: usize = 10;
const RESERVE: usize = 11;

pub(super) const PARAMS: &[Param] = &[
    Param::req(
        "op",
        Kind::Choice(&[
            "read",
            "search",
            "open",
            "click",
            "fill",
            "press",
            "select",
            "scroll",
            "eval",
            "net",
            "screenshot",
        ]),
        "Operation to perform.",
    ),
    Param::opt("url", Kind::Text, "read/open: URL."),
    Param::df(
        "raw",
        Kind::Flag,
        "read: return DOM HTML instead of Markdown.",
        "false",
    ),
    Param::opt(
        "query",
        Kind::Text,
        "search: query, passed through verbatim.",
    ),
    Param::opt(
        "provider",
        Kind::Choice(&[
            "google",
            "bing",
            "duckduckgo",
            "ecosia",
            "startpage",
            "mojeek",
        ]),
        "search: one provider.",
    ),
    Param::df(
        "page",
        Kind::Number,
        "search: target page; fetches pages 1..N.",
        "1",
    ),
    Param::opt(
        "selector",
        Kind::Text,
        "click/fill/select: CSS selector; press: key name.",
    ),
    Param::opt(
        "value",
        Kind::Any,
        "fill/select/eval: string; scroll: pixels.",
    ),
    Param::opt("filter", Kind::Text, "net: URL substring filter."),
    Param::opt("max", Kind::Number, "net: maximum entries, default 20."),
    Param::df(
        "full_page",
        Kind::Flag,
        "screenshot: capture the whole page.",
        "false",
    ),
    Param::df(
        "reserve",
        Kind::Flag,
        "Protect the work tab from idle close during this call.",
        "false",
    ),
];

pub(crate) fn close_session(session: i64) {
    crate::web::close_session(session);
}

pub(super) const TOOL: Tool = Tool {
    name: "web",
    about: "Read, search, and drive a real browser. `read` returns Markdown or DOM HTML; `search` uses one\n\
            provider and merges pages 1..N; `open`/`click`/`fill`/`press`/`select`/`scroll`/`eval` operate the\n\
            work tab; `net` lists performance resource entries; `screenshot` writes a PNG under the system temp\n\
            directory and returns the file location.",
    params: PARAMS,
    run,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let page = args.opt_int(PAGE)?.unwrap_or(1);
    let page = u32::try_from(page).map_err(|_| anyhow!("page must be >= 1"))?;
    let max = match args.opt_int(MAX)? {
        Some(n) => Some(usize::try_from(n).map_err(|_| anyhow!("max must be >= 0"))?),
        None => None,
    };
    let a = crate::web::WebArgs {
        op: args.choice(OP)?,
        url: args.opt_str(URL)?,
        raw: args.flag(RAW)?,
        query: args.opt_str(QUERY)?,
        provider: args.opt_str(PROVIDER)?,
        page,
        selector: args.opt_str(SELECTOR)?,
        value: args.opt_any(VALUE).cloned(),
        filter: args.opt_str(FILTER)?,
        max,
        full_page: args.flag(FULL_PAGE)?,
        reserve: args.flag(RESERVE)?,
    };
    let text = crate::web::run(&a, &env.browser, env.inbox.session)?;
    Ok(ToolOutput::text(text))
}
