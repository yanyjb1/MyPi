//! Card primitives shared by the transcript, the resume picker and the tree.
//!
//! These are the only box-drawing functions in the application. They live here, not under
//! `zone::main::history`, because every zone draws the same frame.

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

pub(crate) fn card_edge(edge: Style, fill: Style, width: usize) -> Line<'static> {
    if width < 8 {
        let dashes = width.saturating_sub(4);
        return Line::from(on_bg(
            vec![
                Span::styled("+-", edge),
                Span::styled("-".repeat(dashes), edge),
                Span::styled("-+", edge),
            ],
            fill,
        ));
    }
    let mid = width - 7;
    Line::from(on_bg(
        vec![
            Span::styled("+-", edge),
            Span::styled("- ", edge),
            Span::styled("-".repeat(mid), edge),
            Span::styled(" -+", edge),
        ],
        fill,
    ))
}

pub(crate) fn card_edge_labeled(
    edge: Style,
    fill: Style,
    width: usize,
    label: Line<'static>,
) -> Line<'static> {
    let budget = width.saturating_sub(7);
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
    let shown = if used > budget {
        clip_spans(spans, budget)
    } else {
        spans
    };
    let used: usize = shown.iter().map(|s| s.content.as_ref().width()).sum();
    let mut out = vec![Span::styled("+- ", edge)];
    out.extend(shown);
    if used > 0 {
        out.push(Span::styled(" ", edge));
    }
    out.push(Span::styled("-".repeat(budget - used), edge));
    out.push(Span::styled(" -+", edge));
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
