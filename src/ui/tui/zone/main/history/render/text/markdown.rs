//! Markdown to rows: headings, lists, tables, quotes and fences.

use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::super::block::Deferred;
use super::super::theme::{HistoryTheme, Token};
use super::highlight;
use crate::ui::tui::text::{char_width, display_width};

#[derive(Default)]
struct TableBuf {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
    cur: Vec<String>,
    cell: Option<String>,
}

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

pub fn render_markdown(
    text: &str,
    t: &HistoryTheme,
    defer: bool,
    width: usize,
) -> (Vec<Line<'static>>, Vec<Deferred>) {
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut deferred: Vec<Deferred> = Vec::new();
    let mut row: Option<Row> = None;
    let mut bold = false;
    let mut italic = false;
    let mut in_heading = false;
    let mut in_code_block: Option<String> = None; // language hint
    let mut code_buf = String::new();
    let mut quote_depth: usize = 0;
    let mut lists: Vec<(bool, u64)> = Vec::new();
    let mut table: Option<TableBuf> = None;

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

    let opts = Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TABLES;
    for ev in Parser::new_ext(text, opts) {
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
                in_heading = true;
                bold = true;
                if depth > 2 {
                    ensure_row!(row);
                    row.as_mut()
                        .unwrap()
                        .push(t.fg_mod(prefix, Token::MdHeading, Modifier::BOLD));
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                bold = false;
                in_heading = false;
                flush(&mut row, &mut out);
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
                out.push(Line::from(
                    t.fg("```".to_string(), Token::MdCodeBlockBorder),
                ));
                in_code_block = Some(lang);
                code_buf.clear();
            }
            Event::End(TagEnd::CodeBlock) => {
                let lang = in_code_block.take().unwrap_or_default();
                let hint = if lang.is_empty() {
                    None
                } else {
                    Some(lang.as_str())
                };
                if defer {
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
                if let Some(c) = table.as_mut().and_then(|tb| tb.cell.as_mut()) {
                    c.push_str(&s);
                } else if in_code_block.is_some() {
                    code_buf.push_str(&s);
                } else if quote_depth > 0 {
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
                            t.fg_style(Token::MdQuote).add_modifier(Modifier::ITALIC),
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
                if let Some(c) = table.as_mut().and_then(|tb| tb.cell.as_mut()) {
                    c.push_str(&t2);
                } else {
                    ensure_row!(row);
                    let r = row.as_mut().unwrap();
                    r.push(t.fg(t2.to_string(), Token::MdCode));
                }
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
            Event::Start(Tag::Table(_alignments)) => {
                flush(&mut row, &mut out);
                table = Some(TableBuf::default());
            }
            Event::End(TagEnd::Table) => {
                if let Some(tb) = table.take() {
                    let lines = render_table(&tb.header, &tb.rows, width, t);
                    if lines.is_empty() {
                        for raw in raw_table_lines(&tb) {
                            out.push(Line::from(Span::styled(raw, t.fg_style(Token::MdCode))));
                        }
                    } else {
                        out.extend(lines);
                        out.push(Line::from(""));
                    }
                }
            }
            Event::Start(Tag::TableHead) => {
                if let Some(tb) = table.as_mut() {
                    tb.cur.clear();
                }
            }
            Event::End(TagEnd::TableHead) => {
                if let Some(tb) = table.as_mut() {
                    tb.header = std::mem::take(&mut tb.cur);
                }
            }
            Event::Start(Tag::TableRow) => {
                if let Some(tb) = table.as_mut() {
                    tb.cur.clear();
                }
            }
            Event::End(TagEnd::TableRow) => {
                if let Some(tb) = table.as_mut() {
                    tb.rows.push(std::mem::take(&mut tb.cur));
                }
            }
            Event::Start(Tag::TableCell) => {
                if let Some(tb) = table.as_mut() {
                    tb.cell = Some(String::new());
                }
            }
            Event::End(TagEnd::TableCell) => {
                if let Some(tb) = table.as_mut()
                    && let Some(text) = tb.cell.take()
                {
                    tb.cur.push(text);
                }
            }
            Event::Rule => {
                flush(&mut row, &mut out);
                out.push(Line::from(Span::styled(
                    "─".repeat(20),
                    t.fg_style(Token::MdHr),
                )));
                out.push(Line::from(""));
            }
            Event::SoftBreak => {
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
    if out.last().is_some_and(|l| l.spans.is_empty()) {
        out.pop();
    }
    (out, deferred)
}

const TABLE_MAX_WORD: usize = 30;

fn longest_word_width(s: &str) -> usize {
    let mut best = 1usize;
    for w in s.split_whitespace() {
        if w.len() >= TABLE_MAX_WORD {
            return TABLE_MAX_WORD;
        }
        best = best.max(display_width(w));
        if best >= TABLE_MAX_WORD {
            return TABLE_MAX_WORD;
        }
    }
    best
}

fn wrap_cell(s: &str, limit: usize) -> Vec<String> {
    let limit = limit.max(1);
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut line_w = 0usize;

    let push_line = |out: &mut Vec<String>, line: &mut String| {
        out.push(std::mem::take(line));
    };

    for word in s.split_whitespace() {
        let word_w = display_width(word);
        let sep = if line.is_empty() { 0 } else { 1 };
        if line_w + sep + word_w <= limit {
            if sep == 1 {
                line.push(' ');
            }
            line.push_str(word);
            line_w += sep + word_w;
            continue;
        }
        if !line.is_empty() {
            push_line(&mut out, &mut line);
            line_w = 0;
        }
        if word_w <= limit {
            line.push_str(word);
            line_w = word_w;
            continue;
        }
        for ch in word.chars() {
            let cw = char_width(ch);
            if line_w + cw > limit && !line.is_empty() {
                push_line(&mut out, &mut line);
                line_w = 0;
            }
            line.push(ch);
            line_w += cw;
        }
    }
    if !line.is_empty() {
        push_line(&mut out, &mut line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn raw_table_lines(tb: &TableBuf) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let join = |cells: &[String]| format!("| {} |", cells.join(" | "));
    if !tb.header.is_empty() {
        out.push(join(&tb.header));
        let rules: Vec<String> = tb.header.iter().map(|_| "---".to_string()).collect();
        out.push(format!("| {} |", rules.join(" | ")));
    }
    for row in &tb.rows {
        out.push(join(row));
    }
    out
}

fn fit_min_widths(natural: &[usize], minw: &[usize], available: usize) -> Vec<usize> {
    let n = natural.len();
    let min_sum: usize = minw.iter().sum();
    if min_sum <= available {
        return minw.to_vec();
    }
    let mut out = vec![1usize; n];
    let remaining = available.saturating_sub(n);
    if remaining > 0 {
        let weights: Vec<usize> = minw.iter().map(|w| w.saturating_sub(1)).collect();
        let total: usize = weights.iter().sum();
        if total > 0 {
            for i in 0..n {
                out[i] += (weights[i] * remaining) / total;
            }
            let allocated: usize = out.iter().sum::<usize>() - n;
            let mut leftover = remaining.saturating_sub(allocated);
            let mut i = 0;
            while leftover > 0 && i < n {
                out[i] += 1;
                leftover -= 1;
                i += 1;
            }
        }
    }
    out
}

fn render_table(
    header: &[String],
    rows: &[Vec<String>],
    width: usize,
    t: &HistoryTheme,
) -> Vec<Line<'static>> {
    let n = header.len();
    if n == 0 {
        return Vec::new();
    }
    let overhead = 3 * n + 1;
    let min_available = n * 2;
    if width < overhead + min_available {
        return Vec::new();
    }
    let available = width - overhead;

    let mut natural = vec![0usize; n];
    let mut minw = vec![1usize; n];
    let note = |cells: &[String], natural: &mut Vec<usize>, minw: &mut Vec<usize>| {
        for (i, c) in cells.iter().enumerate().take(n) {
            natural[i] = natural[i].max(display_width(c));
            minw[i] = minw[i].max(longest_word_width(c));
        }
    };
    note(header, &mut natural, &mut minw);
    for row in rows {
        note(row, &mut natural, &mut minw);
    }

    let natural_sum: usize = natural.iter().sum();
    let widths: Vec<usize> = if natural_sum <= available {
        natural
            .iter()
            .zip(&minw)
            .map(|(a, b)| (*a).max(*b))
            .collect()
    } else {
        let fits = fit_min_widths(&natural, &minw, available);
        let min_sum: usize = fits.iter().sum();
        let total_growth: usize = natural
            .iter()
            .zip(&fits)
            .map(|(a, b)| a.saturating_sub(*b))
            .sum();
        let extra = available.saturating_sub(min_sum);
        let mut out: Vec<usize> = natural
            .iter()
            .zip(&fits)
            .map(|(nat, min)| {
                let delta = nat.saturating_sub(*min);
                let grow = if total_growth > 0 {
                    (delta * extra) / total_growth
                } else {
                    0
                };
                min + grow
            })
            .collect();
        let mut leftover = available.saturating_sub(out.iter().sum::<usize>());
        let mut i = 0;
        while leftover > 0 && i < n {
            if out[i] < natural[i] {
                out[i] += 1;
                leftover -= 1;
            }
            i += 1;
            if i == n && leftover > 0 {
                i = 0;
                if out.iter().zip(&natural).all(|(o, nat)| o >= nat) {
                    break;
                }
            }
        }
        out
    };

    let border = |s: String| Line::from(Span::styled(s, t.fg_style(Token::MdTableBorder)));
    let mut out: Vec<Line<'static>> = Vec::new();

    let mut top = String::from("┌─");
    for (i, w) in widths.iter().enumerate() {
        top.push_str(&"─".repeat(*w));
        top.push_str(if i + 1 == n { "─┐" } else { "─┬─" });
    }
    out.push(border(top));

    let render_row = |cells: &[String], bold: bool| -> Vec<Line<'static>> {
        let wrapped: Vec<Vec<String>> = (0..n)
            .map(|i| wrap_cell(cells.get(i).map(String::as_str).unwrap_or(""), widths[i]))
            .collect();
        let lines = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        (0..lines)
            .map(|li| {
                let mut spans: Vec<Span<'static>> = vec![Span::styled(
                    "│ ".to_string(),
                    t.fg_style(Token::MdTableBorder),
                )];
                for (i, cell) in wrapped.iter().enumerate() {
                    let text = cell.get(li).map(String::as_str).unwrap_or("");
                    let pad = widths[i].saturating_sub(display_width(text));
                    let mut st = Style::new();
                    if bold {
                        st = st.add_modifier(Modifier::BOLD);
                    }
                    let mut padded = String::with_capacity(text.len() + pad);
                    padded.push_str(text);
                    for _ in 0..pad {
                        padded.push(' ');
                    }
                    spans.push(Span::styled(padded, st));
                    spans.push(Span::styled(
                        if i + 1 == n { " │" } else { " │ " }.to_string(),
                        t.fg_style(Token::MdTableBorder),
                    ));
                }
                Line::from(spans)
            })
            .collect()
    };

    out.extend(render_row(header, true));

    let mut sep = String::from("├─");
    for (i, w) in widths.iter().enumerate() {
        sep.push_str(&"─".repeat(*w));
        sep.push_str(if i + 1 == n { "─┤" } else { "─┼─" });
    }

    for row in rows {
        out.push(border(sep.clone()));
        out.extend(render_row(row, false));
    }
    if rows.is_empty() {
        out.push(border(sep));
    }

    let mut bottom = String::from("└─");
    for (i, w) in widths.iter().enumerate() {
        bottom.push_str(&"─".repeat(*w));
        bottom.push_str(if i + 1 == n { "─┘" } else { "─┴─" });
    }
    out.push(border(bottom));
    out
}
