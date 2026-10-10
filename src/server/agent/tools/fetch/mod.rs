//! fetch: read a web page as clean Markdown.

use anyhow::Result;

use super::spec::{Args, Env, Kind, Param, Tool, ToolOutput, ToolProgress};

const URL: usize = 0;
const RAW: usize = 1;

pub(super) const PARAMS: &[Param] = &[
    Param::req(
        "url",
        Kind::Text,
        "URL to read (http/https; a bare domain gets https:// added).",
    ),
    Param::df(
        "raw",
        Kind::Flag,
        "Return the original HTML instead of Markdown. Default false.",
        "false",
    ),
];

pub(super) const TOOL: Tool = Tool {
    name: "fetch",
    about: "Read a web page and return clean Markdown (navigation, ads, and footers stripped).\n\
            For docs, articles, and search-result pages. A page that renders client-side falls back to the\n\
            browser automatically. Output truncates at 24K characters; `raw: true` returns the original HTML.",
    params: PARAMS,
    run,
};

fn run(
    env: &mut Env,
    args: &Args,
    _progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    let a = crate::web::FetchArgs {
        intent: args.intent().to_string(),
        url: args.str(URL)?,
        raw: Some(args.flag(RAW)?),
    };
    let text = crate::web::fetch(&a, &env.browser)?;
    Ok(ToolOutput::text(text))
}
