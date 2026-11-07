//! Matched by the window width, so the layout is computed at use time.

use super::super::row::{Lead, Row, Run};
use super::segment::{Kind, Segment};
use crate::ui::tui::text::{char_width, display_width};
use crate::ui::tui::theme::fixed;

const MAX_WORD: usize = 30;

/// One table at this width. Rows below the first are continuations.
pub(crate) fn rows(seg: &Segment, width: usize) -> Vec<Row> {
    debug_assert_eq!(seg.kind, Kind::Table);
    let table = match split(seg) {
        Some(table) => table,
        None => return raw(seg),
    };
    let rendered = layout(&table, width);
    if rendered.is_empty() {
        return raw(seg);
    }
    let mut out: Vec<Row> = Vec::new();
    for (at, mut row) in rendered.into_iter().enumerate() {
        row.lead = if at == 0 { Lead::First } else { Lead::Wrapped };
        out.push(row);
    }
    out.push(Row::blank());
    out
}

/// One table built from cells, laid at a card's inner width.
pub(crate) fn cells(header: Vec<String>, body: Vec<Vec<String>>, width: usize) -> Vec<Row> {
    layout(&Table { header, body }, width)
}

struct Table {
    header: Vec<String>,
    body: Vec<Vec<String>>,
}

fn split(seg: &Segment) -> Option<Table> {
    let mut lines = seg.source.lines();
    let head = lines.next()?;
    let delimiter = lines.next()?;
    if !delimiter.contains('|') {
        return None;
    }
    let cells = |line: &str| -> Vec<String> {
        let trimmed = line.trim();
        let trimmed = trimmed.strip_prefix('|').unwrap_or(trimmed);
        let trimmed = trimmed.strip_suffix('|').unwrap_or(trimmed);
        trimmed
            .split('|')
            .map(|cell| cell.trim().to_string())
            .collect()
    };
    let header = cells(head);
    if header.is_empty() {
        return None;
    }
    let body: Vec<Vec<String>> = lines.filter(|line| line.contains('|')).map(cells).collect();
    Some(Table { header, body })
}

fn raw(seg: &Segment) -> Vec<Row> {
    seg.source
        .lines()
        .map(|line| {
            Row::of(
                Lead::First,
                vec![Run::fixed(line.to_string(), fixed::MD_CODE)],
            )
        })
        .collect()
}

fn longest_word(text: &str) -> usize {
    let mut best = 1usize;
    for word in text.split_whitespace() {
        if word.len() >= MAX_WORD {
            return MAX_WORD;
        }
        best = best.max(display_width(word));
        if best >= MAX_WORD {
            return MAX_WORD;
        }
    }
    best
}

fn wrap_cell(text: &str, limit: usize) -> Vec<String> {
    let limit = limit.max(1);
    let mut out: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut line_w = 0usize;
    for word in text.split_whitespace() {
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
            out.push(std::mem::take(&mut line));
            line_w = 0;
        }
        if word_w <= limit {
            line.push_str(word);
            line_w = word_w;
            continue;
        }
        for ch in word.chars() {
            let w = char_width(ch);
            if line_w + w > limit && !line.is_empty() {
                out.push(std::mem::take(&mut line));
                line_w = 0;
            }
            line.push(ch);
            line_w += w;
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

fn fit_min(natural: &[usize], minw: &[usize], available: usize) -> Vec<usize> {
    let n = natural.len();
    if minw.iter().sum::<usize>() <= available {
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
            let mut left = remaining.saturating_sub(allocated);
            let mut i = 0;
            while left > 0 && i < n {
                out[i] += 1;
                left -= 1;
                i += 1;
            }
        }
    }
    out
}

fn layout(table: &Table, width: usize) -> Vec<Row> {
    let n = table.header.len();
    if n == 0 {
        return Vec::new();
    }
    let overhead = 3 * n + 1;
    if width < overhead + n * 2 {
        return Vec::new();
    }
    let available = width - overhead;

    let mut natural = vec![0usize; n];
    let mut minw = vec![1usize; n];
    let mut note = |cells: &[String]| {
        for (i, cell) in cells.iter().enumerate().take(n) {
            natural[i] = natural[i].max(display_width(cell));
            minw[i] = minw[i].max(longest_word(cell));
        }
    };
    note(&table.header);
    for row in &table.body {
        note(row);
    }

    let total_natural: usize = natural.iter().sum();
    let widths: Vec<usize> = if total_natural <= available {
        natural
            .iter()
            .zip(&minw)
            .map(|(a, b)| (*a).max(*b))
            .collect()
    } else {
        let fits = fit_min(&natural, &minw, available);
        let min_sum: usize = fits.iter().sum();
        let growth: usize = natural
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
                let grow = if growth > 0 {
                    (delta * extra) / growth
                } else {
                    0
                };
                min + grow
            })
            .collect();
        let mut left = available.saturating_sub(out.iter().sum::<usize>());
        let mut i = 0;
        while left > 0 && i < n {
            if out[i] < natural[i] {
                out[i] += 1;
                left -= 1;
            }
            i += 1;
            if i == n && left > 0 {
                i = 0;
                if out.iter().zip(&natural).all(|(o, nat)| o >= nat) {
                    break;
                }
            }
        }
        out
    };

    let border =
        |text: String| Row::of(Lead::First, vec![Run::fixed(text, fixed::MD_TABLE_BORDER)]);
    let mut out: Vec<Row> = Vec::new();

    let mut top = String::from("┌─");
    for (i, w) in widths.iter().enumerate() {
        top.push_str(&"─".repeat(*w));
        top.push_str(if i + 1 == n { "─┐" } else { "─┬─" });
    }
    out.push(border(top));

    let render = |cells: &[String], bold: bool| -> Vec<Row> {
        let wrapped: Vec<Vec<String>> = (0..n)
            .map(|i| wrap_cell(cells.get(i).map(String::as_str).unwrap_or(""), widths[i]))
            .collect();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);
        (0..height)
            .map(|at| {
                let mut row = Row::new(Lead::First);
                row.push(Run::fixed("│ ".to_string(), fixed::MD_TABLE_BORDER));
                for (i, cell) in wrapped.iter().enumerate() {
                    let text = cell.get(at).map(String::as_str).unwrap_or("");
                    let pad = widths[i].saturating_sub(display_width(text));
                    let mut run = Run::plain(format!("{text}{}", " ".repeat(pad)));
                    if bold {
                        run = run.bold();
                    }
                    row.push(run);
                    row.push(Run::fixed(
                        if i + 1 == n { " │" } else { " │ " }.to_string(),
                        fixed::MD_TABLE_BORDER,
                    ));
                }
                row
            })
            .collect()
    };

    out.extend(render(&table.header, true));

    let mut sep = String::from("├─");
    for (i, w) in widths.iter().enumerate() {
        sep.push_str(&"─".repeat(*w));
        sep.push_str(if i + 1 == n { "─┤" } else { "─┼─" });
    }
    for row in &table.body {
        out.push(border(sep.clone()));
        out.extend(render(row, false));
    }
    if table.body.is_empty() {
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
