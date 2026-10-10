//! browser: drive a real Chromium.

use anyhow::Result;

use super::spec::{Args, Env, Kind, Param, Tool, ToolOutput, ToolProgress};

const COMMAND: usize = 0;
const URL: usize = 1;
const OP: usize = 2;
const SELECTOR: usize = 3;
const VALUE: usize = 4;
const FILTER: usize = 5;
const MAX: usize = 6;
const PATH: usize = 7;
const FULL_PAGE: usize = 8;

pub(super) const PARAMS: &[Param] = &[
    Param::req(
        "command",
        Kind::Choice(&["open", "act", "read", "screenshot"]),
        "What to do.",
    ),
    Param::opt(
        "url",
        Kind::Text,
        "open/act=navigate: URL to open.",
    ),
    Param::opt(
        "op",
        Kind::Choice(&[
            "navigate", "click", "fill", "press", "select", "scroll", "eval", "net",
        ]),
        "The act to perform.",
    ),
    Param::opt(
        "selector",
        Kind::Text,
        "CSS selector (click/fill/select/scroll) or key name (press, e.g. Enter).",
    ),
    Param::opt(
        "value",
        Kind::Any,
        "fill: the text; select: the option value; scroll: pixels; eval: a JS expression.",
    ),
    Param::opt(
        "filter",
        Kind::Text,
        "act=net: keep only requests whose URL contains this substring.",
    ),
    Param::opt("max", Kind::Number, "act=net: max requests returned. Default 20."),
    Param::opt(
        "path",
        Kind::Text,
        "screenshot: PNG output path; read: Markdown output path.",
    ),
    Param::df(
        "full_page",
        Kind::Flag,
        "screenshot: capture the whole page instead of the viewport.",
        "false",
    ),
];

pub(super) const TOOL: Tool = Tool {
    name: "browser",
    about: "Drive a real Chromium browser (Helium, for instance). Four commands: `open` a URL; `act` to interact\n\
            (navigate/click/fill/press/select/scroll/eval/net); `read` the current page as Markdown; `screenshot`\n\
            to a file. For client-rendered SPAs, sites behind a login, and anything needing real clicks, scrolls,\n\
            or network inspection. `act=net` lists the requests the page made — that is how you find the endpoint\n\
            behind a comment section.",
    params: PARAMS,
    run,
};

fn run(
    env: &mut Env,
    args: &Args,
    _progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    let a = crate::web::BrowserArgs {
        intent: args.intent().to_string(),
        command: args.choice(COMMAND)?,
        url: args.opt_str(URL)?,
        op: args.opt_str(OP)?,
        selector: args.opt_str(SELECTOR)?,
        value: args.opt_any(VALUE).cloned(),
        filter: args.opt_str(FILTER)?,
        max: args.opt_int(MAX)?.map(|n| n as usize),
        path: args.opt_str(PATH)?,
        full_page: Some(args.flag(FULL_PAGE)?),
    };
    let text = crate::web::browser(&a, &env.browser)?;
    Ok(ToolOutput::text(text))
}
