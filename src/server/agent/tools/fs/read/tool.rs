// The `read` tool: files by line numbers, directories by name.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, anyhow};

use super::{dir, items};
use crate::server::agent::request::contract::ItemOk;
use crate::server::agent::request::message::ImagePart;
use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::output::ToolOutput;
use crate::server::agent::tools::contract::view::CallRecord;
use crate::server::agent::tools::fs::lang::graph::{self, Loader};
use crate::server::agent::tools::fs::lang::{self, Table};
use crate::server::agent::tools::fs::mask;
use crate::server::agent::tools::fs::skeleton;
use crate::server::agent::tools::fs::sniff;
use crate::server::agent::tools::fs::splice::TOOL as SPLICE;
use crate::server::agent::tools::spec::{Args, Item, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;
use base64::Engine as _;

const FILES: usize = 0;

// A source file's own budget: 800 lines, 256 KiB.
const MAX_LINES_SOURCE: usize = 800;
const MAX_BYTES_SOURCE: usize = 256 * 1024;

// Every other file's budget: 200 lines, 64 KiB.
const MAX_LINES: usize = 200;
const MAX_BYTES: usize = 64 * 1024;

// An image's byte cap: a bigger file fails with the cap named.
const MAX_IMAGE_BYTES: u64 = 4 * 1024 * 1024;

// A file past this size is refused rather than streamed: one line of it could still fill memory.
const MAX_STREAM_BYTES: u64 = 16 * 1024 * 1024;

const FILE_FIELDS: &[Param] = &[
    Param::new("path", Kind::Text, "File or directory path."),
    Param::df(
        "offset",
        Kind::Line,
        "1-based starting line (default: 1). Ignored in `item` mode.",
        "1",
    ),
    Param::new(
        "limit",
        Kind::Number,
        "Max lines to return (capped at 800 for source files, 200 otherwise). Ignored in `item` mode.",
    ),
    Param::new(
        "item",
        Kind::Text,
        "Exact block header from an earlier output (e.g. `impl Shout for Dumb`). Returns every matching block whole with all lines numbered.",
    ),
];

const PARAMS: &[Param] = &[Param::new(
    "files",
    Kind::Objects(FILE_FIELDS),
    "List of targets to read in order. Each item's failure is isolated and does not abort the rest.",
)];

pub(crate) const TOOL: Tool = Tool {
    name: "read",
    about: "Part of the FS family (`read`, `grep`, `splice`/`edit`, `write`). Reads files or directories and grants edit/delete authorization for every file read. Under `splice`, registered languages return structural key line numbers (use `item` for full line-numbered blocks, or `grep` to pinpoint an interior line); read a directory to inspect layout and module dependencies.",
    fields: "Output opens each entry with `==> <path>` (directories include entry counts), and any truncation footer appears at the end.\n\
             - File mode: Supported source files (`Rust`, `Python`, `Go`, `C/C++`, `HTML/CSS/JS`, `R`, `Jupyter Notebook`, `JSON`, `TOML`, `YAML`, `XML`) default to an 800-line / 256 KiB cap; `Markdown` and all other files cap at 200 lines / 64 KiB. When a language supports module graphs (`Rust`, `Python`, `Go`, `C/C++`, `HTML/CSS/JS`, `Markdown`), a neighbourhood view (`exports`, dependencies, callers) is appended once per module or on entry files. Images return raw image data only (<= 4 MiB, requires vision).\n\
             - Directory mode: Lists immediate children (files with line counts) plus one level deeper, collapsing folders with 10+ files into counts. Up-to-date uppercase docs show `(latest)`; user-maintained docs show `(persistent)`.\n\
             - Item mode: When `item` is set, `path` acts as a scope (file or directory). Every block whose header matches `item` is returned in full with every line numbered, ignoring `offset` and `limit`.",
    params: PARAMS,
    run,
    summary,
};

// Read every named path, answering in the call's order.
fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let items = args.objects(FILES)?;
    anyhow::ensure!(!items.is_empty(), "`files` must name at least one path");

    let total = items.len();
    // Resolve first: the module view folds a child that this same call reads.
    let mut targets: Vec<Result<PathBuf>> = Vec::with_capacity(total);
    for item in &items {
        targets.push(resolved(env, item));
    }
    let batch: BTreeSet<PathBuf> = targets
        .iter()
        .filter_map(|one| one.as_ref().ok().cloned())
        .collect();

    let mut out = String::new();
    let mut outcomes: Vec<ItemOk> = Vec::with_capacity(total);
    let mut failed = 0usize;
    let mut images: Vec<ImagePart> = Vec::new();
    for (item, target) in items.iter().zip(targets) {
        let result = match target {
            Ok(path) => match item.opt_str("item") {
                Ok(Some(wanted)) => named(env, item, &path, &wanted),
                Ok(None) => block(env, &batch, item, &path),
                Err(e) => Err(e),
            },
            Err(e) => Err(e),
        };
        match result {
            Ok((text, attached)) => {
                // An image is its part alone: nothing of it is text.
                if !text.is_empty() {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(&text);
                }
                images.extend(attached);
                outcomes.push(ItemOk::ok());
            }
            Err(e) => {
                // One path's failure is that path's own: the rest still run and still answer.
                failed += 1;
                let path = item.text("path").unwrap_or_else(|_| item.label());
                if !out.is_empty() {
                    out.push('\n');
                }
                let _ = write!(out, "{}[error: {e:#}]", header(&path));
                outcomes.push(ItemOk::failed(format!("{e:#}")));
            }
        }
    }
    let mut produced = ToolOutput::text(out);
    produced.items = outcomes;
    produced.images = images;
    if failed > 0 {
        produced = produced.failure(format!("{failed} of {total} paths failed"));
    }
    Ok(produced)
}

fn resolved(env: &Env, item: &Item) -> Result<PathBuf> {
    let raw = item.text("path")?;
    anyhow::ensure!(!raw.trim().is_empty(), "`{}` has no path", item.label());
    env.workspace.resolve_any(&raw, true)
}

fn named(
    env: &mut Env,
    item: &Item,
    path: &Path,
    wanted: &str,
) -> Result<(String, Vec<ImagePart>)> {
    let raw = item.text("path")?;
    let mut text = header(&raw);
    // The scope replaced the window: the fields that shape a window are spent, and saying so is cheaper than a refused call.
    if item.gave("offset") || item.gave("limit") {
        text.push_str("[i] item mode: `offset` and `limit` do not apply\n");
    }
    text.push_str(&items::answer(env, path, wanted)?);
    Ok((text, Vec::new()))
}

// One answer's line of a batch, and its own signpost when alone: every answer names its path.
pub(super) fn header(raw: &str) -> String {
    format!("==> {raw}\n")
}

// A body under its own header.
pub(super) fn titled(raw: &str, text: &str) -> String {
    format!("{}{text}", header(raw))
}

// An image's answer, when the file is one and this session's model can see: the bytes as a part, and no text at all. A non-image, or a model without vision, answers `None`.
fn image(env: &Env, path: &Path) -> Result<Option<ImagePart>> {
    let Some(format) = sniff::detect(path)? else {
        return Ok(None);
    };
    anyhow::ensure!(
        env.vision,
        "the session's model takes no images: `{}` answers text only",
        path.display()
    );
    let meta = std::fs::metadata(path).context("failed to read image")?;
    anyhow::ensure!(
        meta.len() <= MAX_IMAGE_BYTES,
        "the image is {} KiB; the cap is {} KiB",
        meta.len().div_ceil(1024),
        MAX_IMAGE_BYTES / 1024,
    );
    let bytes = std::fs::read(path).context("failed to read image")?;
    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(Some(ImagePart {
        format: format.to_string(),
        data,
    }))
}

// The lines one item may take: the file's own cap, or what the call named when that is smaller.
fn window(item: &Item, cap: usize) -> Result<usize> {
    match item.opt_int("limit")? {
        None => Ok(cap),
        Some(n) => {
            anyhow::ensure!(n >= 1, "`{}.limit` must be at least 1", item.label());
            Ok((n as usize).min(cap))
        }
    }
}

// One path's answer: a listing, or a body with its module view.
fn block(
    env: &mut Env,
    batch: &BTreeSet<PathBuf>,
    item: &Item,
    path: &Path,
) -> Result<(String, Vec<ImagePart>)> {
    let raw = item.text("path")?;
    let offset = item.line("offset")?;

    if path.is_dir() {
        let (counts, body) = dir::listing(path, &env.workspace.cwd(), &env.persistent_docs)?;
        return Ok((titled(&format!("{raw}  {counts}"), &body), Vec::new()));
    }

    if let Some(image) = image(env, path)? {
        return Ok((String::new(), vec![image]));
    }

    let splice = env.allows(SPLICE.name);
    let language = lang::owner(path);
    let source = language.is_some_and(|one| one.source);
    // A sorted language is read whole so its key lines can be numbered.
    let limit = window(item, if source { MAX_LINES_SOURCE } else { MAX_LINES })?;
    let max_bytes = if source { MAX_BYTES_SOURCE } else { MAX_BYTES };
    let whole = if language.is_some() {
        graph::loader::text(path)
    } else {
        None
    };
    let body = if let Some(text) = &whole {
        let lines: Vec<&str> = text.lines().collect();
        let total = lines.len();
        let table = lang::table(path, text);
        let numbers = if splice {
            Numbers::Table(&table)
        } else {
            Numbers::None
        };
        assemble(
            offset,
            total,
            limit,
            lines.into_iter().skip(offset.saturating_sub(1)),
            &numbers,
            max_bytes,
        )
    } else {
        let streamed = streamed(path, offset, limit, max_bytes)?;
        let numbers = if splice { Numbers::All } else { Numbers::None };
        assemble(
            offset,
            streamed.total,
            limit,
            streamed.lines.iter().map(String::as_str),
            &numbers,
            max_bytes,
        )
    };

    if let Some(text) = &whole {
        env.seen.stamp(path.to_path_buf(), text);
    }

    // The body, then the module view, then the footer: what was left out comes last.
    let mut text = body.text;

    if let Some(facts) = lang::hub(path)
        && let Some(unit) = graph::unit(facts, path)
    {
        let loader = Loader::new();
        let is_entry = unit.entry.as_deref() == Some(path);
        if is_entry {
            text.push_str(&sketch(facts, &loader, path, batch));
            env.seen.dirs.insert(unit.place);
        } else if env.seen.dirs.insert(unit.place) {
            text.push_str(&sketch(facts, &loader, path, batch));
        }
    }

    if let Some(status) = &body.status {
        text.push_str(status);
        text.push('\n');
    }

    Ok((titled(&raw, &text), Vec::new()))
}

// One file's neighbourhood, printed and bounded on its own.
fn sketch(facts: &lang::Hub, loader: &Loader, path: &Path, batch: &BTreeSet<PathBuf>) -> String {
    let around = graph::build(facts, loader, path);
    let mut text = String::new();
    for row in graph::render(facts, &around, loader, batch) {
        if let Some(heading) = row.heading {
            text.push_str(&heading);
            text.push('\n');
        }
        text.push_str(&row.text);
        text.push('\n');
    }
    text.push('\n');
    text
}

// How a file's lines are prefixed, if at all.
enum Numbers<'a> {
    // No numbers: a session without `splice`.
    None,
    // A file no language sorts, under `splice`: every line carries one.
    All,
    // A sorted file under `splice`: only the key lines do.
    Table(&'a Table),
}

impl Numbers<'_> {
    // Do any lines carry a number? A gutter is only reserved when they do.
    fn on(&self) -> bool {
        !matches!(self, Numbers::None)
    }

    fn numbered(&self, at: usize) -> bool {
        match self {
            Numbers::None => false,
            Numbers::All => true,
            Numbers::Table(table) => table.key(at.saturating_sub(1)),
        }
    }
}

// A body and, when the file continues past it, the line that says so.
struct Body {
    text: String,
    status: Option<String>,
}

// The window's lines, prefixed by a gutter as wide as the file's own numbers, capped by bytes. The gutter is `total`'s digit count plus one pipe, so a numbered line and an unnumbered one start in the same column and an empty line costs a newline and nothing else.
fn assemble<'a>(
    first: usize,
    total: usize,
    limit: usize,
    lines: impl Iterator<Item = &'a str>,
    numbers: &Numbers<'_>,
    max_bytes: usize,
) -> Body {
    let width = skeleton::width(total);
    let mut text = String::new();
    let mut bytes = 0usize;
    let mut shown = 0usize;
    let mut by_bytes = false;
    // The file line this window reaches last: a fold spends lines without drawing them, and the next offset counts the file, not the page.
    let mut last = first.saturating_sub(1);
    let file: Vec<&str> = lines.collect();
    let drawn = mask::block(&file);
    for (index, held) in drawn.iter().enumerate() {
        if shown >= limit {
            break;
        }
        let at = first + index;
        // A folded run draws its own first line and spends the rest: the numbers it skips are the point, and they lie behind the window all the same.
        let mask::Shown::Line(line) = held else {
            last = at;
            continue;
        };
        let line = line.as_ref();
        let blank = line.trim().is_empty();
        let number = numbers.numbered(at);
        let gutter = if blank || !numbers.on() { 0 } else { width + 1 };
        let room = max_bytes.saturating_sub(bytes + gutter);
        if line.len() + 1 > room {
            // The line is longer than the bytes left: keep what fits, and let the next read start at it again so no character is lost.
            let keep = floor_boundary(line, room.saturating_sub(1));
            if keep > 0 {
                push(&mut text, at, &line[..keep], number, numbers.on(), width);
                by_bytes = true;
            }
            break;
        }
        push(&mut text, at, line, number, numbers.on(), width);
        bytes += gutter + line.len() + 1;
        shown += 1;
        last = at;
    }
    let status = (last < total).then(|| {
        let cap = if by_bytes {
            format!("{} KiB cap; ", max_bytes / 1024)
        } else {
            String::new()
        };
        format!("[{cap}{last} of {total} lines; next offset {}]", last + 1)
    });
    Body { text, status }
}

// One line: its number, or the same width of blanks, then the pipe that ends the gutter.
fn push(out: &mut String, at: usize, line: &str, number: bool, on: bool, width: usize) {
    if !on {
        // Without numbers the body is the file itself, so a line of spaces stays a line of spaces.
        out.push_str(line);
        out.push('\n');
        return;
    }
    if line.trim().is_empty() {
        out.push('\n');
        return;
    }
    let mark = number.then_some(at);
    out.push_str(&skeleton::numbered(width, mark, line));
    out.push('\n');
}

struct Streamed {
    total: usize,
    // The window's lines, unprefixed: the gutter needs the total, which is only known at the end.
    lines: Vec<String>,
}

// A file of any other kind, streamed line by line so a huge one never lands in memory whole.
// Blocking: the whole file is read here, capped at MAX_STREAM_BYTES.
fn streamed(path: &Path, offset: usize, limit: usize, max_bytes: usize) -> Result<Streamed> {
    use std::io::BufRead as _;

    let meta = std::fs::metadata(path).context("failed to read file size")?;
    anyhow::ensure!(
        meta.len() <= MAX_STREAM_BYTES,
        "the file is {} MiB; `read` refuses files above {} MiB",
        meta.len().div_ceil(1024 * 1024),
        MAX_STREAM_BYTES / (1024 * 1024),
    );
    let file = std::fs::File::open(path).context("failed to open file")?;
    let mut reader = std::io::BufReader::new(file);
    let mut buf: Vec<u8> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let mut kept = 0usize;
    let mut total = 0usize;
    loop {
        buf.clear();
        if reader.read_until(b'\n', &mut buf)? == 0 {
            break;
        }
        total += 1;
        if total < offset || lines.len() >= limit || kept >= max_bytes {
            continue;
        }
        let line = std::str::from_utf8(&buf).map_err(|_| anyhow!("not a valid UTF-8 text file"))?;
        let line = line.trim_end_matches(['\n', '\r']);
        kept += line.len() + 1;
        lines.push(line.to_string());
    }
    Ok(Streamed { total, lines })
}

// The largest character boundary within the first `bytes` bytes.
fn floor_boundary(line: &str, bytes: usize) -> usize {
    let mut end = bytes.min(line.len());
    while end > 0 && !line.is_char_boundary(end) {
        end -= 1;
    }
    end
}

// One line for a history listing: the paths, the first named and the rest counted.
fn summary(call: &CallRecord) -> String {
    let paths = summary::paths(call, "files");
    let first = paths.first().cloned().unwrap_or_default();
    summary::counted(&first, paths.len())
}
