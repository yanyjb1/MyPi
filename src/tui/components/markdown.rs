//! Markdown -> ratatui rows.
//!
//! `pulldown-cmark` is a pull parser: feed text, receive an event stream
//! (Start(Paragraph) / Text(...) / Rule / ...); we fold that stream into styled rows
//! ourselves. Only the subset visible in a terminal is supported:
//! headings, bold, italic, inline code, code blocks, lists, quotes, rules.
//! The rest (tables, footnotes...) degrades to plain text.
//!
//! Entry points: user / assistant / tool_result (markdown-capable) messages go
//! through [`render_markdown`]; tool_request and Plain views do not.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::tui::theme::Palette;

// Render markdown text into rows. Empty text yields nothing.
pub fn render_markdown(text: &str, p: &Palette) -> Vec<Line<'static>> {
    let mut out: Vec<Line<'static>> = Vec::new();
    // Current row's style state: set on entering code blocks/emphasis, reset on exit
    let mut bold = false;
    let mut italic = false;
    let mut in_code_block = false;
    let mut in_quote = false;
    let mut list_depth: usize = 0;

    for ev in Parser::new_ext(text, Options::ENABLE_STRIKETHROUGH) {
        match ev {
            Event::Start(Tag::Paragraph) => out.push(Line::from(Vec::new())),
            Event::Start(Tag::Heading { level, .. }) => {
                bold = true;
                out.push(Line::from(Span::styled(
                    heading_prefix(&level),
                    Style::new().fg(p.accent).add_modifier(Modifier::BOLD),
                )));
            }
            Event::End(TagEnd::Heading(_)) => {
                bold = false;
            }
            Event::Start(Tag::CodeBlock(_)) => in_code_block = true,
            Event::End(TagEnd::CodeBlock) => {
                in_code_block = false;
                out.push(Line::from(""));
            }
            Event::Start(Tag::List(_)) => list_depth += 1,
            Event::End(TagEnd::List(_)) => list_depth -= 1,
            Event::Start(Tag::Item) => {
                // List items: indent + bullet opens a row; following Text events append to it
                out.push(Line::from(Span::styled(
                    format!("{}• ", "  ".repeat(list_depth.saturating_sub(1))),
                    Style::new().fg(p.accent),
                )));
            }
            Event::End(TagEnd::Item) => {}
            Event::Start(Tag::BlockQuote(_)) => in_quote = true,
            Event::End(TagEnd::BlockQuote(_)) => in_quote = false,
            Event::Start(Tag::Emphasis) => italic = true,
            Event::End(TagEnd::Emphasis) => italic = false,
            Event::Start(Tag::Strong) => bold = true,
            Event::End(TagEnd::Strong) => bold = false,
            Event::Start(Tag::Strikethrough) => {}
            Event::End(TagEnd::Strikethrough) => {}
            Event::Rule => out.push(Line::styled(
                "─".repeat(20),
                Style::new().fg(p.muted),
            )),
            Event::SoftBreak => out.push(Line::from("")),
            Event::HardBreak => out.push(Line::from("")),
            Event::Text(t) => {
                let s = t.to_string();
                if in_code_block {
                    // Code blocks: indented and dimmed, split per row
                    for part in s.split('\n') {
                        out.push(Line::styled(
                            format!("  {part}"),
                            Style::new().fg(p.muted),
                        ));
                    }
                    // split leaves a trailing empty segment (text ending in \n); drop it
                    if s.ends_with('\n') {
                        out.pop();
                    }
                } else if in_quote {
                    if out.is_empty() {
                        out.push(Line::from(Vec::new()));
                    }
                    if let Some(last) = out.last_mut() {
                        last.spans.push(Span::styled(
                            format!("▏ {s}"),
                            Style::new().fg(p.muted).add_modifier(Modifier::ITALIC),
                        ));
                    }
                } else {
                    let mut style = Style::new();
                    if bold {
                        style = style.add_modifier(Modifier::BOLD);
                    }
                    if italic {
                        style = style.add_modifier(Modifier::ITALIC);
                    }
                    // Append to the current row (multiple Text events in one paragraph share it)
                    if out.is_empty() {
                        out.push(Line::from(Vec::new()));
                    }
                    if let Some(last) = out.last_mut() {
                        last.spans.push(Span::styled(s, style));
                    }
                }
            }
            Event::Code(t) => {
                // Inline code: cyan, **appended to the current row**.
                //
                // An earlier version pushed a fresh line here, so a sentence
                // like "用 `edit` 改 `main.rs`" broke into one row per code
                // span — the paragraph fell apart the moment it mentioned a
                // filename. Inline code is inline; it belongs to its row.
                if out.is_empty() {
                    out.push(Line::from(Vec::new()));
                }
                if let Some(last) = out.last_mut() {
                    last.spans.push(Span::styled(t.to_string(), Style::new().fg(Color::Cyan)));
                }
            }
            Event::Html(_) | Event::InlineHtml(_) => {}
            Event::FootnoteReference(_) | Event::TaskListMarker(_) => {}
            Event::InlineMath(_) | Event::DisplayMath(_) => {}
            _ => {}
        }
    }
    // Drop a single trailing blank row (only a paragraph separator)
    if out.last().is_some_and(|l| l.spans.is_empty()) {
        out.pop();
    }
    out
}

// Heading level -> prefix decoration (the number of #), for readability.
fn heading_prefix(level: &HeadingLevel) -> String {
    match level {
        HeadingLevel::H1 => "# ".into(),
        HeadingLevel::H2 => "## ".into(),
        _ => "### ".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bold_and_plain_mix() {
        let p = Palette::default();
        let lines = render_markdown("普通 **加粗** 结束", &p);
        assert_eq!(lines.len(), 1, "单段一行: {lines:?}");
        let spans = &lines[0].spans;
        assert_eq!(spans[0].content, "普通 ");
        assert_eq!(spans[0].style.add_modifier, Modifier::empty());
        // pulldown splits "**bold**" into separate Text events
        let bold_span = spans.iter().find(|s| s.content == "加粗").expect("应有加粗 span");
        assert!(bold_span.style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn code_block_is_indented_and_muted() {
        let p = Palette::default();
        let lines = render_markdown("前文\n\n```rust\nlet x = 1;\nlet y = 2;\n```\n\n后文", &p);
        let joined: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>())
            .collect();
        assert!(joined.iter().any(|l| l == "  let x = 1;"), "{joined:?}");
        assert!(joined.iter().any(|l| l == "  let y = 2;"), "{joined:?}");
    }

    #[test]
    fn heading_is_bold() {
        let p = Palette::default();
        let lines = render_markdown("## 标题", &p);
        assert!(!lines.is_empty());
        let has_bold = lines[0]
            .spans
            .iter()
            .any(|s| s.style.add_modifier.contains(Modifier::BOLD));
        assert!(has_bold, "标题应加粗: {:?}", lines[0]);
    }

    #[test]
    fn quote_gets_bar_prefix() {
        let p = Palette::default();
        let lines = render_markdown("> 引用内容", &p);
        let joined = lines[0].spans.iter().map(|s| s.content.to_string()).collect::<String>();
        assert!(joined.contains("▏"), "{joined}");
    }

    #[test]
    fn empty_input_gives_empty_output() {
        let p = Palette::default();
        assert!(render_markdown("", &p).is_empty());
    }
}
