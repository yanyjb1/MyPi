//! The `read` tool: files by line numbers.

use anyhow::{Context as _, Result, anyhow};

use crate::server::agent::request::contract::ItemOk;
use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::output::ToolOutput;
use crate::server::agent::tools::contract::view::CallRecord;
use crate::server::agent::tools::spec::{Args, Item, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;

const FILES: usize = 0;

/// The most lines one file may take; past it the read stops and the footer names the next offset.
const MAX_LINES: usize = 200;

/// The most bytes one file may take; a line over it is cut and the footer points at bash.
const MAX_BYTES: usize = 64 * 1024;

const FILE_FIELDS: &[Param] = &[
    Param::new("path", Kind::Text, "File path."),
    Param::df("offset", Kind::Line, "First line to read (1-based).", "1"),
    Param::df(
        "limit",
        Kind::Number,
        "Max lines this file may take.",
        "200",
    ),
];

const PARAMS: &[Param] = &[Param::new(
    "files",
    Kind::Objects(FILE_FIELDS),
    "One item per file, answered in this order. One item's failure never stops the ones after it.",
)];

pub(crate) const TOOL: Tool = Tool {
    name: "read",
    about: "Read files with line numbers, so later turns can cite exact lines. MUST read before \
            editing; NEVER guess the current text. Files only.",
    fields: "files is the only field a call needs: one item per file, with `path` and its own \
             `offset` (1-based, default 1) and `limit` (default 200). Each file gets its own \
             200-line and 64 KiB budget.",
    params: PARAMS,
    run,
    summary,
};

/// Read every named file, with line numbers, so later turns can cite exact lines.
fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let items = args.objects(FILES)?;
    anyhow::ensure!(!items.is_empty(), "`files` must name at least one file");

    let total = items.len();
    let mut out = String::new();
    let mut outcomes: Vec<ItemOk> = Vec::with_capacity(total);
    let mut failed = 0usize;
    for item in &items {
        match block(env, item, total > 1) {
            Ok(text) => {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&text);
                outcomes.push(ItemOk::ok());
            }
            Err(e) => {
                // One file's failure is that file's own: the rest still run and still answer.
                if total == 1 {
                    return Err(e);
                }
                failed += 1;
                let path = item.text("path").unwrap_or_else(|_| item.label());
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(&format!("==> {path}\n[error: {e:#}]"));
                outcomes.push(ItemOk::failed(format!("{e:#}")));
            }
        }
    }
    let mut produced = ToolOutput::text(out);
    produced.items = outcomes;
    if failed > 0 {
        produced = produced.failure(format!("{failed} of {total} files failed"));
    }
    Ok(produced)
}

/// One file's numbered body, plus its header when the call names more than one file. Each file gets
/// its own line and byte budget: a big file never shortens the ones beside it.
fn block(env: &Env, item: &Item, headed: bool) -> Result<String> {
    use std::io::BufRead as _;

    let path = item.text("path")?;
    let offset = item.line("offset")?;
    let limit = item.int("limit")?;
    anyhow::ensure!(limit >= 1, "`{}.limit` must be at least 1", item.label());
    let limit = limit as usize;

    let p = env.workspace.resolve(&path, true)?;
    let max_lines = limit.min(MAX_LINES);

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
        let line = std::str::from_utf8(&buf).map_err(|_| anyhow!("not a valid UTF-8 text file"))?;
        let line = line.trim_end_matches(['\n', '\r']);
        if shown >= max_lines {
            more = true;
            break;
        }
        let room = MAX_BYTES.saturating_sub(bytes);
        if line.len() > room {
            // Over the byte cap: keep the whole characters that still fit, then stop.
            let cut = floor_boundary(line, room);
            if cut > 0 {
                out.push_str(&format!("{:>5}\t{}\n", line_no, &line[..cut]));
                shown += 1;
            }
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
        if by_bytes {
            out.push_str(&format!(
                "\n[cut at the {MAX_BYTES} byte cap; the rest of this file needs bash]"
            ));
        } else if limit > max_lines {
            out.push_str(&format!(
                "\n[{max_lines} lines at most per file; lines {}-{}, continue with offset={}]",
                offset,
                offset + shown.saturating_sub(1),
                offset + shown
            ));
        } else {
            out.push_str(&format!(
                "\n[more lines; continue with offset={}]",
                offset + shown
            ));
        }
    }

    Ok(if headed {
        format!("==> {path}\n{out}")
    } else {
        out
    })
}

/// The largest character boundary within the first `bytes` bytes.
fn floor_boundary(line: &str, bytes: usize) -> usize {
    let mut end = bytes.min(line.len());
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// One line for a history listing: the files, the first named and the rest counted.
fn summary(call: &CallRecord) -> String {
    let paths = summary::paths(call, "files");
    let first = paths.first().cloned().unwrap_or_default();
    summary::clip(&summary::counted(&first, paths.len()), 96)
}
