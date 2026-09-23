//! Syntax highlighting for tool cards, on top of `syntect`.
//!
//! Two entry points:
//! - [`highlight`] — colour one code block given a syntax hint;
//! - [`language_for`] — pick that hint from a file path or a tool name.
//!
//! Everything is lazily built once per process: `SyntaxSet::load_defaults_*`
//! deserialises a few MB of definitions, so it must never run per frame.
//! `syntect`'s default colour scheme is picked to stay legible on a black
//! card; unknown languages degrade to plain text rather than failing.

use std::sync::LazyLock;

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(SyntaxSet::load_defaults_newlines);

// `base16-ocean.dark`: a low-saturation scheme chosen so highlighted code
// does not fight the rest of the UI. Falls back to the first bundled theme
// if the name ever disappears upstream.
static THEME: LazyLock<Theme> = LazyLock::new(|| {
    let set = ThemeSet::load_defaults();
    set.themes
        .get("base16-ocean.dark")
        .or_else(|| set.themes.values().next())
        .cloned()
        .expect("syntect ships at least one default theme")
});

/// Highlight `code` using the given syntax name (`"bash"`, `"python"`, …).
///
/// Unknown syntaxes and parse hiccups fall back to unstyled lines — a
/// tool card must always render.
pub fn highlight(code: &str, language: Option<&str>) -> Vec<Line<'static>> {
    let Some(language) = language else {
        return plain(code);
    };
    let ss = &*SYNTAXES;
    let Some(syntax) = ss
        .find_syntax_by_token(language)
        .or_else(|| ss.find_syntax_by_extension(language))
    else {
        return plain(code);
    };
    let mut h = HighlightLines::new(syntax, &THEME);
    let mut out = Vec::new();
    for line in code.lines() {
        match h.highlight_line(line, ss) {
            Ok(ranges) => {
                let spans: Vec<Span<'static>> = ranges
                    .into_iter()
                    .map(|(style, text)| {
                        let c = style.foreground;
                        Span::styled(
                            text.to_string(),
                            Style::new().fg(Color::Rgb(c.r, c.g, c.b)),
                        )
                    })
                    .collect();
                out.push(if spans.is_empty() {
                    Line::from("")
                } else {
                    Line::from(spans)
                });
            }
            // One bad line should not lose the rest of the block.
            Err(_) => out.push(Line::from(line.to_string())),
        }
    }
    if out.is_empty() {
        out.push(Line::from(""));
    }
    out
}

fn plain(code: &str) -> Vec<Line<'static>> {
    code.lines().map(|l| Line::from(l.to_string())).collect()
}

/// Syntax hint for a path (extension only — the file is never read).
pub fn language_for_path(path: &str) -> Option<&'static str> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())?
        .to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => "rs",
        "py" | "pyi" => "py",
        "js" | "mjs" | "cjs" | "jsx" => "js",
        "ts" | "tsx" => "ts",
        "go" => "go",
        "c" | "h" => "c",
        "cc" | "cpp" | "hpp" | "cxx" => "cpp",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "json" => "json",
        "sh" | "bash" | "zsh" => "sh",
        "md" | "markdown" => "md",
        "html" | "htm" => "html",
        "css" => "css",
        "sql" => "sql",
        "java" => "java",
        "rb" => "rb",
        "lua" => "lua",
        "nix" => "nix",
        _ => return None,
    })
}

/// Syntax hint for a tool's own payload language.
///
/// `bash` payloads are shell; edit-family payloads are file content (the
/// caller passes the path so the extension decides).
pub fn language_for_tool(tool: &str) -> Option<&'static str> {
    match tool {
        "bash" => Some("sh"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line]) -> String {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn bash_block_keeps_every_line() {
        let code = "cd /tmp\nls -la | grep foo";
        let out = highlight(code, Some("sh"));
        assert_eq!(out.len(), 2, "每个源行必须对应一行输出");
        assert_eq!(text(&out), code, "高亮不得增删字符");
    }

    #[test]
    fn highlighted_output_is_styled() {
        let out = highlight("let x = 1;", Some("rs"));
        let styled = out
            .iter()
            .flat_map(|l| l.spans.iter())
            .any(|s| s.style.fg.is_some());
        assert!(styled, "已知语法必须着色");
    }

    #[test]
    fn unknown_language_degrades_to_plain() {
        let out = highlight("whatever", None);
        assert_eq!(text(&out), "whatever");
        assert!(out[0].spans.iter().all(|s| s.style.fg.is_none()), "未知语法不着色");
    }

    #[test]
    fn paths_map_to_syntaxes() {
        assert_eq!(language_for_path("a/b/main.rs"), Some("rs"));
        assert_eq!(language_for_path("x.py"), Some("py"));
        assert_eq!(language_for_path("README"), None);
        assert_eq!(language_for_tool("bash"), Some("sh"));
        assert_eq!(language_for_tool("edit"), None);
    }

    #[test]
    fn empty_input_yields_one_empty_line() {
        assert_eq!(highlight("", Some("sh")), vec![Line::from("")]);
    }
}
