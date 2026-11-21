// Box-drawing primitives for the full-screen pages: a dashed edge, a dashed edge with a label
// laid over its middle, one framed row, and the clip that keeps a row inside it.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::ui::tui::text::take_width;

pub(crate) fn on_bg(spans: Vec<Span<'static>>, bg: Style) -> Vec<Span<'static>> {
    spans
        .into_iter()
        .map(|sp| Span::styled(sp.content, bg.patch(sp.style)))
        .collect()
}

// A dash run of exactly `n` columns: a dash, a blank, and so on, so the run reads as dashed.
// Equal lengths look equal, which is what keeps the two sides of a labeled edge matched.
pub(crate) fn dashes(n: usize) -> String {
    let mut out = String::with_capacity(n);
    for i in 0..n {
        out.push(if i % 2 == 0 { '-' } else { ' ' });
    }
    out
}

pub(crate) fn card_edge(edge: Style, fill: Style, width: usize) -> Line<'static> {
    if width < 2 {
        return Line::from(on_bg(vec![Span::styled("-".repeat(width), edge)], fill));
    }
    let line = format!("+{}+", dashes(width - 2));
    Line::from(on_bg(vec![Span::styled(line, edge)], fill))
}

pub(crate) fn card_edge_labeled(
    edge: Style,
    fill: Style,
    width: usize,
    label: Line<'static>,
) -> Line<'static> {
    let budget = width.saturating_sub(4);
    if budget == 0 {
        return card_edge(edge, fill, width);
    }
    let line_style = label.style;
    let spans: Vec<Span<'static>> = label
        .spans
        .into_iter()
        .map(|sp| Span::styled(sp.content, line_style.patch(sp.style)))
        .collect();
    let used: usize = spans.iter().map(|s| s.content.as_ref().width()).sum();
    if used == 0 {
        return card_edge(edge, fill, width);
    }
    let shown = if used > budget {
        clip_spans(spans, budget)
    } else {
        spans
    };
    let used: usize = shown.iter().map(|s| s.content.as_ref().width()).sum();
    if used == 0 {
        return card_edge(edge, fill, width);
    }
    // The two runs split what is left, so the label sits mid-edge and the sides match.
    let rest = width - 4 - used;
    let lead = rest / 2;
    let trail = rest - lead;
    let mut out = vec![Span::styled(format!("+{} ", dashes(lead)), edge)];
    out.extend(shown);
    out.push(Span::styled(format!(" {}+", dashes(trail)), edge));
    Line::from(on_bg(out, fill))
}

pub(crate) fn card_row(
    content: Line<'static>,
    edge: Style,
    body_bg: Style,
    width: usize,
) -> Line<'static> {
    let inner = width.saturating_sub(4);
    let clean_style = content.style;
    let content = Line::from(
        content
            .spans
            .into_iter()
            .map(|sp| Span::styled(crate::ansi::strip_ansi(&sp.content), sp.style))
            .collect::<Vec<_>>(),
    )
    .style(clean_style);
    let mut spans = vec![Span::styled("| ", edge)];
    let line_style = content.style;
    let content: Vec<Span<'static>> = content
        .spans
        .into_iter()
        .map(|sp| Span::styled(sp.content, line_style.patch(sp.style)))
        .collect();
    let used: usize = content.iter().map(|s| s.content.as_ref().width()).sum();
    let clipped: Vec<Span<'static>> = if used > inner {
        clip_spans(content, inner)
    } else {
        content
    };
    let used: usize = clipped.iter().map(|s| s.content.as_ref().width()).sum();
    spans.extend(clipped);
    if used < inner {
        spans.push(Span::styled(" ".repeat(inner - used), body_bg));
    }
    spans.push(Span::styled(" |", edge));
    Line::from(on_bg(spans, body_bg))
}

pub(crate) fn clip_spans(spans: Vec<Span<'static>>, max: usize) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut used = 0usize;
    for sp in spans {
        let w = sp.content.as_ref().width();
        if used + w <= max {
            used += w;
            out.push(sp);
        } else {
            let keep = max.saturating_sub(used);
            if keep > 0 {
                let (cut, _) = take_width(&sp.content, keep);
                out.push(Span::styled(cut, sp.style));
            }
            break;
        }
    }
    out
}
