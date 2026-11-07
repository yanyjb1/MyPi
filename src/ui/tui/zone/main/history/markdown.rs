//! Markdown to natural rows: headings, lists, quotes, inline emphasis.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::Modifier;

use crate::ui::tui::theme::fixed;
use crate::ui::tui::zone::main::history::row::{Ink, Lead, Row, Run};

/// One segment's markdown, one row per source line. No width involved.
pub(crate) fn rows(text: &str) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::new();
    let mut row: Option<Row> = None;
    let mut bold = false;
    let mut italic = false;
    let mut heading = false;
    let mut quote = 0usize;
    let mut lists: Vec<(bool, u64)> = Vec::new();

    macro_rules! ensure {
        () => {
            if row.is_none() {
                row = Some(Row::new(Lead::First));
            }
        };
    }
    macro_rules! flush {
        () => {
            if let Some(r) = row.take() {
                out.push(r);
            }
        };
    }

    let opts = Options::ENABLE_STRIKETHROUGH;
    for event in Parser::new_ext(text, opts) {
        match event {
            Event::End(TagEnd::Paragraph) => {
                flush!();
            }
            Event::Start(Tag::Heading { level, .. }) => {
                flush!();
                let depth = match level {
                    pulldown_cmark::HeadingLevel::H1 => 1,
                    pulldown_cmark::HeadingLevel::H2 => 2,
                    _ => 3,
                };
                heading = true;
                bold = true;
                if depth > 2 {
                    ensure!();
                    row.as_mut().expect("row just ensured").push(Run::fixed(
                        format!("{} ", "#".repeat(depth)),
                        fixed::MD_HEADING,
                    ));
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                bold = false;
                heading = false;
                flush!();
            }
            Event::Text(text) => {
                ensure!();
                let target = row.as_mut().expect("row just ensured");
                if quote > 0 {
                    for part in text.split('\n').filter(|part| !part.is_empty()) {
                        if target.runs.is_empty() {
                            target.push(Run::fixed("▏ ", fixed::MD_QUOTE_BORDER));
                        }
                        target.push(
                            Run::with(
                                part.to_string(),
                                Some(Ink::Fixed(fixed::MD_QUOTE)),
                                Modifier::ITALIC,
                            )
                            .bold_if(bold),
                        );
                    }
                    continue;
                }
                let mut attrs = Modifier::empty();
                if bold {
                    attrs |= Modifier::BOLD;
                }
                if italic {
                    attrs |= Modifier::ITALIC;
                }
                let ink = if heading {
                    Some(Ink::Fixed(fixed::MD_HEADING))
                } else {
                    None
                };
                target.push(Run::with(text.to_string(), ink, attrs));
            }
            Event::Code(code) => {
                ensure!();
                row.as_mut()
                    .expect("row just ensured")
                    .push(Run::fixed(code.to_string(), fixed::MD_CODE));
            }
            Event::Start(Tag::List(start)) => {
                flush!();
                lists.push((start.is_some(), start.unwrap_or(1)));
            }
            Event::End(TagEnd::List(_)) => {
                flush!();
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                flush!();
                let depth = lists.len().saturating_sub(1);
                let slot = lists.last_mut().expect("item inside list");
                let bullet = if slot.0 {
                    let n = slot.1;
                    slot.1 += 1;
                    format!("{n}. ")
                } else {
                    "- ".to_string()
                };
                let mut line = Row::new(Lead::First);
                line.push(Run::fixed(
                    format!("{}{bullet}", "  ".repeat(depth)),
                    fixed::MD_LIST_BULLET,
                ));
                row = Some(line);
            }
            Event::End(TagEnd::Item) => flush!(),
            Event::Start(Tag::BlockQuote(_)) => {
                flush!();
                quote += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                flush!();
                quote = quote.saturating_sub(1);
            }
            Event::Start(Tag::Emphasis) => italic = true,
            Event::End(TagEnd::Emphasis) => italic = false,
            Event::Start(Tag::Strong) => bold = true,
            Event::End(TagEnd::Strong) => bold = false,
            Event::Rule => {
                flush!();
                out.push(Row::of(
                    Lead::First,
                    vec![Run::fixed("─".repeat(20), fixed::MD_HR)],
                ));
            }
            Event::SoftBreak | Event::HardBreak => flush!(),
            _ => {}
        }
    }
    flush!();
    if out.last().is_some_and(Row::is_blank) {
        out.pop();
    }
    out
}

trait BoldIf {
    fn bold_if(self, on: bool) -> Self;
}

impl BoldIf for Run {
    fn bold_if(self, on: bool) -> Run {
        if on { self.bold() } else { self }
    }
}
