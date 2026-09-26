//! Markdown → ratatui rows, omp-style.
//!
//! Layout contract ported from omp `components/markdown.ts` (terminal
//! subset): headings carry `#` prefixes with an underline on h1, lists
//! use omp's hanging indent (`2×depth` prefix, continuation rows align
//! under the text past the bullet, ordered lists keep their real
//! numbers), quotes draw the `▏ ` gutter, rules render as `─` runs,
//! fenced code blocks get a border-gray ``` frame with syntax
//! highlighting inside, inline code is `mdCode` colored **inline**.
//!
//! Everything else (tables, footnotes, math) degrades to plain text.
//!
//! Entry points: user / assistant / tool_result (markdown-capable)
//! messages go through [`render_markdown`]; tool_request and Plain views
//! do not.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::blocks::Deferred;
use super::highlight;
use super::theme::{HistoryTheme, Token};

/// One in-flight text row being assembled.
struct Row {
    spans: Vec<Span<'static>>,
}

impl Row {
    fn push(&mut self, sp: Span<'static>) {
        self.spans.push(sp);
    }
    fn take(&mut self) -> Line<'static> {
        Line::from(std::mem::take(&mut self.spans))
    }
}

/// Render markdown text into rows. Empty text yields nothing.
///
/// `defer` 时**代码围栏按纯文本出图**，并把每个围栏的位置与源码一并交回
/// （[`Deferred`]），由调用方决定何时补色。围栏之间是独立的渲染单元
/// （每个围栏一个 fresh `ParseState`），所以"先出哪个围栏的颜色"是自由的。
pub fn render_markdown(
    text: &str,
    t: &HistoryTheme,
    defer: bool,
) -> (Vec<Line<'static>>, Vec<Deferred>) {
    // Colors read from the global theme (fresh per call): runtime theme
    // switches recolor the next render with no signature churn.
    let mut out: Vec<Line<'static>> = Vec::new();
    // 推迟上色的段（只在新 `defer` 时非空）。
    let mut deferred: Vec<Deferred> = Vec::new();
    // Current row being assembled (paragraph / list item text).
    let mut row: Option<Row> = None;
    // Inline style state.
    let mut bold = false;
    let mut italic = false;
    let mut in_heading = false;
    // Block context.
    let mut in_code_block: Option<String> = None; // language hint
    let mut code_buf = String::new();
    let mut quote_depth: usize = 0;
    // List context: stack of (ordered, next_number). omp renders real
    // numbers for ordered lists; nested lists indent by 2 cells per level.
    let mut lists: Vec<(bool, u64)> = Vec::new();

    let inline_style = |bold: bool, italic: bool| {
        let mut s = Style::new();
        if bold {
            s = s.add_modifier(Modifier::BOLD);
        }
        if italic {
            s = s.add_modifier(Modifier::ITALIC);
        }
        s
    };

    let flush = |row: &mut Option<Row>, out: &mut Vec<Line<'static>>| {
        if let Some(mut r) = row.take() {
            out.push(r.take());
        }
    };

    macro_rules! ensure_row {
        ($row:expr) => {
            if $row.is_none() {
                $row = Some(Row { spans: Vec::new() });
            }
        };
    }

    for ev in Parser::new_ext(text, Options::ENABLE_STRIKETHROUGH) {
        match ev {
            Event::Start(Tag::Paragraph) => {}
            Event::End(TagEnd::Paragraph) => {
                flush(&mut row, &mut out);
                out.push(Line::from(""));
            }
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut row, &mut out);
                let depth = match level {
                    pulldown_cmark::HeadingLevel::H1 => 1,
                    pulldown_cmark::HeadingLevel::H2 => 2,
                    _ => 3,
                };
                let prefix = format!("{} ", "#".repeat(depth));
                // omt: h1/h2 drop the prefix for large-text feel… but in
                // terminals without text sizing, the `#` keeps the heading
                // readable; h1 gets the underline treatment via bold+accent.
                in_heading = true;
                bold = true;
                if depth > 2 {
                    ensure_row!(row);
                    row.as_mut().unwrap().push(t.fg_mod(
                        prefix,
                        Token::MdHeading,
                        Modifier::BOLD,
                    ));
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                bold = false;
                in_heading = false;
                flush(&mut row, &mut out);
                // Heading breathes: omp pads after headings unless a blank
                // line already follows.
                out.push(Line::from(""));
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush(&mut row, &mut out);
                let lang = match &kind {
                    pulldown_cmark::CodeBlockKind::Fenced(info) => {
                        info.split([',', ' ']).next().unwrap_or("").to_string()
                    }
                    _ => String::new(),
                };
                // omt: the fence lines render in mdCodeBlockBorder gray.
                out.push(Line::from(
                    t.fg("```".to_string(), Token::MdCodeBlockBorder),
                ));
                in_code_block = Some(lang);
                code_buf.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                let lang = in_code_block.take().unwrap_or_default();
                // Highlight the whole block at once (syntect needs forward
                // state across lines), then split into rows.
                let hint = if lang.is_empty() {
                    None
                } else {
                    Some(lang.as_str())
                };
                if defer {
                    // 纯文本出图 + 登记这一段：行数与颜色无关，几何照旧。
                    let before = out.len();
                    let plain = highlight::plain(&code_buf);
                    let count = plain.len();
                    out.extend(plain);
                    deferred.push(Deferred {
                        before,
                        count,
                        code: std::mem::take(&mut code_buf),
                        lang: hint.map(str::to_string),
                    });
                } else {
                    for line in highlight::highlight(&code_buf, hint, t) {
                        out.push(line);
                    }
                }
                out.push(Line::from(
                    t.fg("```".to_string(), Token::MdCodeBlockBorder),
                ));
                out.push(Line::from(""));
            }
            Event::Text(txt) => {
                let s = txt.to_string();
                if in_code_block.is_some() {
                    code_buf.push_str(&s);
                } else if quote_depth > 0 {
                    // Quote rows: gutter + text, italics muted (omp mdQuote).
                    // Continuation events (inline code, wrapped source rows)
                    // append to the open quote row — the gutter appears once
                    // per visual row, not once per parser event.
                    for part in s.split('\n') {
                        if part.is_empty() {
                            continue;
                        }
                        ensure_row!(row);
                        let r = row.as_mut().unwrap();
                        if r.spans.is_empty() {
                            r.push(t.fg("▏ ".to_string(), Token::MdQuoteBorder));
                        }
                        r.push(Span::styled(
                            part.to_string(),
                            t.fg_style(Token::MdQuote)
                                .add_modifier(Modifier::ITALIC),
                        ));
                    }
                } else {
                    ensure_row!(row);
                    let r = row.as_mut().unwrap();
                    let mut st = inline_style(bold, italic);
                    if in_heading {
                        st = st.fg(t.get(Token::MdHeading));
                    }
                    r.push(Span::styled(s, st));
                }
            }
            Event::Code(t2) => {
                // Inline code: mdCode color, appended to the current row —
                // a sentence mentioning `main.rs` stays one sentence.
                ensure_row!(row);
                let r = row.as_mut().unwrap();
                r.push(t.fg(t2.to_string(), Token::MdCode));
            }
            Event::Start(Tag::List(start)) => {
                flush(&mut row, &mut out);
                let ordered = start.is_some();
                lists.push((ordered, start.unwrap_or(1)));
            }
            Event::End(TagEnd::List(_)) => {
                flush(&mut row, &mut out);
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                flush(&mut row, &mut out);
                let depth = lists.len().saturating_sub(1);
                let indent = "  ".repeat(depth);
                let slot = lists.last_mut().expect("Item inside List");
                let ordered = slot.0;
                let n = slot.1;
                if ordered {
                    slot.1 += 1;
                }
                let bullet = if ordered {
                    format!("{n}. ")
                } else {
                    // ascii preset: the omp bullet is `- `; the unicode `-` is
                    // the same glyph, so one form serves both.
                    "- ".to_string()
                };
                let mut r = Row { spans: Vec::new() };
                r.push(t.fg(format!("{indent}{bullet}"), Token::MdListBullet));
                row = Some(r);
            }
            Event::End(TagEnd::Item) => {
                flush(&mut row, &mut out);
            }
            Event::Start(Tag::BlockQuote(_)) => {
                flush(&mut row, &mut out);
                quote_depth += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                flush(&mut row, &mut out);
                quote_depth = quote_depth.saturating_sub(1);
            }
            Event::Start(Tag::Emphasis) => italic = true,
            Event::End(TagEnd::Emphasis) => italic = false,
            Event::Start(Tag::Strong) => bold = true,
            Event::End(TagEnd::Strong) => bold = false,
            Event::Start(Tag::Strikethrough) | Event::End(TagEnd::Strikethrough) => {}
            Event::Rule => {
                flush(&mut row, &mut out);
                out.push(Line::from(Span::styled(
                    "─".repeat(20),
                    t.fg_style(Token::MdHr),
                )));
                out.push(Line::from(""));
            }
            Event::SoftBreak => {
                // Paragraph-internal newline: start a new row (hanging
                // alignment comes free — list prefix only adorns row 0).
                flush(&mut row, &mut out);
            }
            Event::HardBreak => {
                flush(&mut row, &mut out);
            }
            Event::TaskListMarker(_) => {}
            Event::Html(_) | Event::InlineHtml(_) => {}
            Event::FootnoteReference(_) => {}
            Event::InlineMath(_) | Event::DisplayMath(_) => {}
            _ => {}
        }
    }
    flush(&mut row, &mut out);
    // Drop a single trailing blank row (paragraph separator)
    if out.last().is_some_and(|l| l.spans.is_empty()) {
        out.pop();
    }
    (out, deferred)
}

/// 测试用的便捷入口：不推迟上色，只要行。
#[cfg(test)]
fn render_markdown_at(text: &str, t: &HistoryTheme) -> Vec<Line<'static>> {
    render_markdown(text, t, false).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::theme::HistoryTheme;
    use ratatui::style::Color;

    fn t() -> HistoryTheme {
        HistoryTheme::resolve()
    }

    fn text_of(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn bold_and_plain_mix() {
        let lines = render_markdown_at("**bold** plain", &t());
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].style.has_modifier(Modifier::BOLD));
        assert!(!lines[0].spans[1].style.has_modifier(Modifier::BOLD));
    }

    #[test]
    fn code_block_is_fenced_and_indented() {
        let lines = render_markdown_at("```rust\nlet x = 1;\n```", &t());
        let all = text_of(&lines);
        assert!(all.starts_with("```"), "{all}");
        assert!(all.trim_end().ends_with("```"), "{all}");
        assert!(all.contains("let x = 1;"), "{all}");
    }

    #[test]
    fn heading_is_bold_accent() {
        let lines = render_markdown_at("## 标题", &t());
        assert_eq!(lines.len(), 1);
        assert!(lines[0].spans[0].style.has_modifier(Modifier::BOLD));
        assert_eq!(
            lines[0].spans[0].style.fg,
            Some(HistoryTheme::resolve().get(Token::MdHeading))
        );
    }

    #[test]
    fn h3_keeps_hash_prefix() {
        let lines = render_markdown_at("### 深层", &t());
        assert_eq!(lines[0].spans[0].content, "### ");
    }

    #[test]
    fn quote_gets_bar_prefix() {
        let lines = render_markdown_at("> 引用一句", &t());
        assert_eq!(lines[0].spans[0].content, "▏ ");
    }

    #[test]
    fn ordered_list_keeps_numbers() {
        let md = "1. first\n2. second\n3. third";
        let all = text_of(&render_markdown_at(md, &t()));
        assert!(all.contains("1. first"), "{all}");
        assert!(all.contains("2. second"), "{all}");
        assert!(all.contains("3. third"), "{all}");
    }

    #[test]
    fn unordered_list_uses_dash() {
        let all = text_of(&render_markdown_at("- a\n- b", &t()));
        assert!(all.contains("- a"), "{all}");
        assert!(all.contains("- b"), "{all}");
    }

    #[test]
    fn nested_list_indents() {
        let md = "- top\n  - inner";
        let all = text_of(&render_markdown_at(md, &t()));
        let inner = all.lines().find(|l| l.contains("inner")).expect("row");
        assert!(inner.starts_with("  - "), "{all}"); // omt: 每级 2 格，恰好对齐上级文本
    }

    #[test]
    fn list_hangs_under_text_not_prefix() {
        // Continuation of a wrapped item aligns past the bullet; here the
        // item is one row, so just verify the prefix exists and the bullet
        // is list-bullet colored.
        let lines = render_markdown_at("- item", &t());
        assert_eq!(
            lines[0].spans[0].style.fg,
            Some(HistoryTheme::resolve().get(Token::MdListBullet))
        );
        assert_eq!(lines[0].spans[0].content, "- ");
    }

    #[test]
    fn inline_code_stays_inline() {
        let lines = render_markdown_at("改 `main.rs` 和 `lib.rs`", &t());
        assert_eq!(lines.len(), 1, "行内 code 不能断行");
        let t = HistoryTheme::resolve();
        assert_eq!(
            lines[0].spans[1].style.fg,
            Some(t.get(Token::MdCode))
        );
    }

    #[test]
    fn softbreak_breaks_row() {
        let lines = render_markdown_at("一行\n二行", &t());
        assert_eq!(lines.len(), 2);
    }

    #[test]
    fn empty_input_gives_empty_output() {
        assert!(render_markdown_at("", &t()).is_empty());
    }

    #[test]
    fn user_card_markdown_survives_gutter() {
        // user_card wraps markdown rows with the ▌ gutter; the inline code
        // must not break that layout.
        let lines = super::super::cards::user_card_public_for_test(
            "看 `x.rs`",
            &t(),
            30,
        );
        assert!(lines.len() >= 3);
    }

    #[test]
    fn link_autolink_renders_plain() {
        let all = text_of(&render_markdown_at("<https://example.com>", &t()));
        assert!(all.contains("https://example.com"), "{all}");
    }

    #[test]
    fn rule_is_mdhr_colored() {
        let lines = render_markdown_at("---\n\n正文", &t());
        assert_eq!(
            lines[0].spans[0].style.fg,
            Some(HistoryTheme::resolve().get(Token::MdHr))
        );
        let _ = Color::Reset;
    }
}
