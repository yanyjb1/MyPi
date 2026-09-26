//! Card scaffolding + tool cards — the bordered, true-black visual system.
//!
//! `pub(crate)`: 全屏页面（`zone::resume`）也吃这套框线，所以整应用
//! 只有一处画框——第二套边框约定是被禁止的。
//!
//! One set of primitives (`card_edge` / `card_row` / `on_bg` / `pad_to`)
//! draws every card; the tool views (exchange / request / result / payload)
//! compose them. Restyling the cards happens here and nowhere else.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

use super::theme::{HistoryTheme, Token};
use super::tools;
use unicode_width::UnicodeWidthStr;

// ---------------------------------------------------------------------------
// card scaffolding
// ---------------------------------------------------------------------------

// Pad `line` out to `width` display cells with `fill`.
//
// Every card body goes through this: a background only covers the cells it
// actually paints, so an unpadded row would end mid-card.
pub(super) fn pad_to(mut line: Line<'static>, width: usize, fill: Style) -> Line<'static> {
    let used: usize = line.spans.iter().map(|s| s.content.as_ref().width()).sum();
    if used < width {
        line.spans
            .push(Span::styled(" ".repeat(width - used), fill));
    }
    line
}

// Give every cell of a card row the card's background.
//
// A background only covers the cells it paints, and painting it on the
// padding alone left the text sitting on the terminal default — so a card
// looked black in the gaps and *not* black behind the words. Every span
// (frame included) goes through here.
pub(super) fn on_bg(spans: Vec<Span<'static>>, bg: Style) -> Vec<Span<'static>> {
    spans
        .into_iter()
        .map(|sp| Span::styled(sp.content, bg.patch(sp.style)))
        .collect()
}

// A card's horizontal edge, exactly `width` cells:
//
//     +- - -------…------- -+
//     └┬┘└┬┘        └┬┘└┬┘
//      │  │          │  └── right break: one space, one dash, corner
//      │  └──────────────  left break: one dash, one space
//      └─────────────────── corner
//
// The two single-dash **breaks** exist for ligature-capable fonts: Sarasa and
// friends fuse a consecutive run of dashes into one glyph and draw it narrower
// than the cells we reserved, so the frame stopped short of the body. Breaking
// the run at both ends keeps the corners from touching the middle, which is
// enough to defeat the fusion — while the long middle stretch still reads as
// one continuous rule (the earlier attempt to space *every* dash looked like a
// dotted line and was rejected).
//
// Both ends give up one cell to a break. Degenerate widths fall back to a plain
// run so the row can never exceed the budget.
pub(crate) fn card_edge(edge: Style, fill: Style, width: usize) -> Line<'static> {
    // `+-` + `-` + ` ` + mid + ` ` + `-` + `-+` = 7 cells of frame/scaffolding.
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

// A card's horizontal edge **with a label embedded in it**, exactly `width`
// cells:
//
//     +- [ok] bash: 跑测试 --------- +
//     └┬┘└──────────┬───────────┘└┬┘
//      │             │             └── right break, as in `card_edge`
//      │             └──────────────── label took the place of the rule
//      └────────────────────────────── corner
//
// The label is clipped to the space the rule would have used; if not even a
// cell of label fits, this degrades to the plain edge.
pub(crate) fn card_edge_labeled(
    edge: Style,
    fill: Style,
    width: usize,
    label: Line<'static>,
) -> Line<'static> {
    // `+-` + ` ` + label + ` ` + rule + ` -+`
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

// One body row: `| ` + content + padding + ` |`.
pub(crate) fn card_row(
    content: Line<'static>,
    edge: Style,
    body_bg: Style,
    width: usize,
) -> Line<'static> {
    let inner = width.saturating_sub(4);
    // Defensive: never let stray escape bytes reach the terminal from inside a
    // card. The tools strip their own output, but file contents and rows read
    // back from an older DB can still carry them.
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
    // `Line::styled(x, s)` puts `s` on the **line**, not on its spans, so a
    // caller that styles a whole row (the diff's red/green) would otherwise
    // lose that color the moment we take the spans apart. Fold it in first.
    let line_style = content.style;
    let content: Vec<Span<'static>> = content
        .spans
        .into_iter()
        .map(|sp| Span::styled(sp.content, line_style.patch(sp.style)))
        .collect();
    // Reserve the right border before padding the content.
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
    // Frame, text and padding all sit on the card's black.
    Line::from(on_bg(spans, body_bg))
}

// Cut spans at `max` display cells (keeps a card row inside its borders).
pub(super) fn clip_spans(spans: Vec<Span<'static>>, max: usize) -> Vec<Span<'static>> {
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
                let (cut, _) = crate::tui::text::take_width(&sp.content, keep);
                out.push(Span::styled(cut, sp.style));
            }
            break;
        }
    }
    out
}

// ---------------------------------------------------------------------------

// tool cards
// ---------------------------------------------------------------------------

/// 一个工具块一张卡：header 行在顶边上，正文在中间，结果接在同一张卡里。
///
/// 画法全部交给 [`tools::ToolBlock`]：这里只负责把它的行塞进 ASCII 框
/// （`+- label ----+` / `| content |`）。框的边框色和底色由块状态决定 ——
/// 成功 dim 灰、失败红、进行中强调色。
///
/// `args` 为空 = 只拿到结果没有调用（被中断的调用）；`result` 为 `None`
/// = 只拿到调用没有结果（还在跑）。
pub(super) fn tool_card(
    name: &str,
    args: &str,
    result: Option<tools::ToolOutcome<'_>>,
    t: &HistoryTheme,
    expanded: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let block = tools::ToolBlock {
        name,
        args,
        result,
        expanded,
    };
    let edge = block.edge_style(t);
    let bg = block.bg_style(t);
    // 段落流：header 那条顶边就是第一段的分节线，往后的段落各起一条
    // （带标签的段落把标签嵌进这条线里）。
    let mut out = vec![card_edge_labeled(edge, bg, width, block.header(t))];
    for (i, section) in block.sections(t).into_iter().enumerate() {
        // 带标签的段落永远自己起一条（header 那条是顶边，不是它的）；无标签
        // 的段落靠上一条横杠收进来，只有它不是第一段时才补一条。
        if i > 0 || section.label.is_some() {
            out.push(match section.label {
                Some(label) => card_edge_labeled(
                    edge,
                    bg,
                    width,
                    Line::from(t.fg(label, Token::ToolTitle)),
                ),
                None => card_edge(edge, bg, width),
            });
        }
        for line in section.lines {
            out.push(card_row(line, edge, bg, width));
        }
    }
    out.push(card_edge(edge, bg, width));
    out
}

/// Ctrl+O（展开）是否改变这个块的渲染 —— 缓存据此决定存一份还是两份。
///
/// 工具交上来的 `details` 也参与判断：diff 形状的结果永远不折叠，所以是
/// 单变体，而这件事只有 `details` 说了算（`kind == "diff"`）。
pub(crate) fn exchange_has_two_states(
    name: &str,
    ok: bool,
    result: &str,
    details: Option<&serde_json::Value>,
) -> bool {
    tools::ToolBlock {
        name,
        args: "",
        result: Some(tools::ToolOutcome {
            text: result,
            ok,
            details,
            duration_ms: 0,
        }),
        expanded: false,
    }
    .has_two_states()
}

// Estimate the rendered visual line count (used to compute scrolling).

// User message: a card on the true-black background, white text.
//
// The black is the palette's truecolor black (`0,0,0`), never the ANSI
// indexed black — indexed black is what terminals map to gray, which is
// exactly the look we are avoiding here.
pub(super) fn user_card(content: &str, t: &HistoryTheme, width: usize) -> Vec<Line<'static>> {
    let bg = Style::new().bg(t.get(Token::UserCardBg));
    let mut out = Vec::new();
    // Breathing room: a blank row above and below, still on the accent gutter
    // and the true-black background, so the card reads as one solid block.
    out.push(pad_to(
        Line::from(Span::styled("▌ ", Style::new().bg(t.get(Token::UserCardBg)).fg(t.get(Token::UserCardBar)))),
        width,
        bg,
    ));
    // 用户卡的每个 span 颜色都会被下面覆盖成统一的黑底白字：在这里高亮是
    // 白烧的钱。纯文本出图，且不需要谁来补色。
    for line in super::markdown::render_markdown(content, t, true).0 {
        // Force the card's own foreground/background: markdown may have
        // decided on a color for a code span, but a user message is
        // uniformly black-on-… white-on-black.
        let mut spans = vec![Span::styled("▌ ", Style::new().bg(t.get(Token::UserCardBg)).fg(t.get(Token::UserCardBar)))];
        for sp in line.spans {
            let body_fg = match t.get(Token::UserCardText) {
                ratatui::style::Color::Reset => Color::White,
                c => c,
            };
            spans.push(Span::styled(
                sp.content,
                Style::new().bg(t.get(Token::UserCardBg)).fg(body_fg),
            ));
        }
        out.push(pad_to(Line::from(spans), width, bg));
    }
    out.push(pad_to(
        Line::from(Span::styled("▌ ", Style::new().bg(t.get(Token::UserCardBg)).fg(t.get(Token::UserCardBar)))),
        width,
        bg,
    ));
    out
}

/// Test hook: markdown-in-user-card integration lives in markdown.rs tests.
#[cfg(test)]
pub(crate) fn user_card_public_for_test(
    content: &str,
    t: &HistoryTheme,
    width: usize,
) -> Vec<Line<'static>> {
    user_card(content, t, width)
}

// (the trailing blank row between nodes is `section_gap`, added by the caller)
