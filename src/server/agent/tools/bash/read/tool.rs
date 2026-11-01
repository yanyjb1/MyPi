//! The `read` tool: a file by line numbers.

use anyhow::{Context as _, Result, anyhow};

use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::output::ToolOutput;
use crate::server::agent::tools::contract::view::CallRecord;
use crate::server::agent::tools::spec::{Args, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;

const PATH: usize = 0;
const OFFSET: usize = 1;
const LIMIT: usize = 2;
const DEFAULT_LINES: usize = 500;

const PARAMS: &[Param] = &[
    Param::req("path", Kind::Text, "File path. `./` and `../` allowed."),
    Param::df(
        "offset",
        Kind::Line,
        "First line to read (1-based, default 1). The output footer names the offset for the next call.",
        "1",
    ),
    Param::opt(
        "limit",
        Kind::Number,
        "Max lines this call; defaults to 500 and is capped by the configured output line limit.",
    ),
];

pub(crate) const TOOL: Tool = Tool {
    name: "read",
    about: "Read a file with line numbers so later turns can cite exact lines. MUST read before editing; NEVER guess the current text.\n\
            Files only, never directories; only inside the workspace (the session's start directory) and the temp directory.\n\
            Past the line cap the output stops early and its footer names the offset to continue from.",
    params: PARAMS,
    run,
    summary: Some(summary),
};

/// Read a file, with line numbers, so later turns can cite exact lines. **Capped on purpose**:
/// reading stops at the cap with a footer naming the next `offset`, so memory costs the cap.
fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    use std::io::BufRead as _;

    let path = args.str(PATH)?;
    let offset = args.line(OFFSET)?;
    let limit = match args.opt_int(LIMIT)? {
        Some(0) => return Err(anyhow!("`limit` must be at least 1")),
        Some(n) => Some(n as usize),
        None => None,
    };

    let limits = env.limits;
    let p = env.workspace.resolve(&path, true)?;
    let max_lines = limit.unwrap_or(DEFAULT_LINES).min(limits.lines).max(1);

    let file = std::fs::File::open(&p).context("failed to open file")?;
    let mut rdr = std::io::BufReader::new(file);
    let mut buf: Vec<u8> = Vec::new();
    let mut out = String::new();
    let mut line_no = 0usize;
    let mut shown = 0usize;
    let mut bytes = 0usize;
    let mut more = false;
    let mut by_bytes = false;
    loop {
        buf.clear();
        if rdr.read_until(b'\n', &mut buf)? == 0 {
            break;
        }
        line_no += 1;
        if line_no < offset {
            continue;
        }
        let line = std::str::from_utf8(&buf)
            .map_err(|_| anyhow!("not a UTF-8 text file (line {line_no} does not decode)"))?;
        let line = line.trim_end_matches(['\n', '\r']);
        if shown >= max_lines {
            more = true;
            break;
        }
        if shown > 0 && bytes + line.len() > limits.bytes {
            more = true;
            by_bytes = true;
            break;
        }
        out.push_str(&format!("{:>5}\t{line}\n", line_no));
        bytes += line.len();
        shown += 1;
    }

    if shown == 0 {
        out = if offset > 1 {
            format!("(no content from line {offset} on)")
        } else {
            "(empty file)".to_string()
        };
    } else if more {
        out.push_str(&format!(
            "\n[more lines; continue with offset={}]",
            offset + shown
        ));
        if by_bytes {
            out.push_str(&format!(
                " (hit the {} byte cap before EOF; total line count unknown)",
                limits.bytes
            ));
        }
    }

    Ok(ToolOutput::text(out))
}

/// One line for a history listing: which file, from which line.
fn summary(call: &CallRecord) -> String {
    let path = summary::field(call, "path").unwrap_or_default();
    match summary::field(call, "offset").filter(|o| o != "1") {
        Some(offset) => summary::clip(&format!("{path} from line {offset}"), 96),
        None => summary::clip(&path, 96),
    }
}
