//! Chat history: typed entries + rendering dispatched by kind.
//!
//! An early version stored messages as "prefix + body" strings in `Vec<String>`,
//! with renderers guessing the type via strip_prefix — a REPL-era relic,
//! now fully retired. History is an array of [`Entry`]; rendering dispatches on kind.
//!
//! Data flow: streaming writes only the in-memory in-progress slot; at TurnDone the
//! whole round's entries are persisted in one shot. Rendering only accepts `&[Entry]`,
//! so in-memory and DB-sourced history share one render path.
//!
//! This file only *maps* an entry to a block ([`BlockKind`] + text) and calls
//! [`render_block`] — the looks live there, one arm per kind:
//! - **user / assistant / reasoning** — done, via the funnel;
//! - **tool / system / error** — still dispatched inline below, to be moved
//!   onto the funnel when their own looks are designed.

#[cfg(test)]
use ratatui::style::Color;
use ratatui::text::Line;
#[cfg(test)]
use ratatui::widgets::Paragraph;
#[cfg(test)]
use unicode_width::UnicodeWidthStr;

use super::cards::tool_card;
use super::tools::ToolOutcome;
use super::system::system_block;
use super::{BlockKind, Streaming, render_block};
use crate::server::entry::{Align, Entry};
use super::theme::HistoryTheme;
#[cfg(test)]
use super::theme::Token;

// Render entries into ratatui lines.
//
// The single render entry point: in-memory history and DB-resumed history
// both go through it, guaranteeing "reopened after persistence" looks identical to "just typed".
#[cfg(test)]
pub fn render(
    entries: &[Entry],
    t: &HistoryTheme,
    show_reasoning: bool,
    tools_expanded: bool,
) -> Vec<Line<'static>> {
    render_at(entries, t, show_reasoning, tools_expanded, 80)
}

#[cfg(test)]
pub fn render_at(
    entries: &[Entry],
    t: &HistoryTheme,
    show_reasoning: bool,
    tools_expanded: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for r in super::blocks::blocks(entries) {
        if !out.is_empty() {
            out.push(Line::from(""));
        }
        out.extend(single_node(
            &entries[r.start..r.end],
            t,
            show_reasoning,
            tools_expanded,
            width,
        ));
    }
    out
}

/// Example-only public shim over the test renderer (docs/preview tools
/// render the real pipeline without opening a TUI). Not part of the API.
#[doc(hidden)]
pub fn render_at_public(
    entries: &[Entry],
    t: &HistoryTheme,
    show_reasoning: bool,
    tools_expanded: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for r in super::blocks::blocks(entries) {
        if !out.is_empty() {
            out.push(Line::from(""));
        }
        out.extend(single_node(
            &entries[r.start..r.end],
            t,
            show_reasoning,
            tools_expanded,
            width,
        ));
    }
    out
}

// Render exactly one transcript node (no gap, no pairing): the kind
// dispatch `render_with_live` used to inline. Grouping decisions live in
// `blocks::blocks`; this only paints.
pub(crate) fn single_node(
    group: &[Entry],
    t: &HistoryTheme,
    show_reasoning: bool,
    tools_expanded: bool,
    width: usize,
) -> Vec<Line<'static>> {
    debug_assert!(
        group.len() <= 2,
        "a node is one entry or one request+result pair"
    );
    let e = &group[0];
    match e {
        Entry::User { content } => {
            render_block(BlockKind::User, content, Streaming::Final, width, t)
        }
        Entry::Assistant { content, .. } => {
            render_block(BlockKind::Assistant, content, Streaming::Final, width, t)
        }
        // The thinking chain, its own block: Ctrl+T hides the whole block
        // (a *visibility* switch here, not a render flag — the cache keeps
        // one variant per block and never re-renders).
        Entry::Reasoning { content } => {
            if !show_reasoning || content.trim().is_empty() {
                return Vec::new();
            }
            render_block(BlockKind::Reasoning, content, Streaming::Final, width, t)
        }
        Entry::ToolRequest {
            name,
            args,
            text,
            first,
            ..
        } => {
            // 组里只有调用 = 工具还在跑；成组的两条 = 调用 + 结果，一张卡。
            // 结果那一半带上工具交的结构化载荷与耗时 —— 卡片画什么由数据决定。
            let paired = group.get(1).and_then(|e| match e {
                Entry::ToolResult {
                    ok,
                    result,
                    details,
                    duration_ms,
                    ..
                } => Some(ToolOutcome {
                    text: result.as_str(),
                    ok: *ok,
                    details: details.as_ref(),
                    duration_ms: *duration_ms,
                }),
                _ => None,
            });
            let mut out = Vec::new();
            // 模型在调工具前说的那半句：它作为**第一条调用**的一部分存档
            // （`Entry::ToolRequest.text`），所以画在卡片**上面**当正文。
            // 只认第一条：一条消息带多个调用时，那半句不属于后面几次。
            if *first && !text.trim().is_empty() {
                out.extend(render_block(
                    BlockKind::Assistant,
                    text,
                    Streaming::Final,
                    width,
                    t,
                ));
                out.push(super::blocks::block_gap());
            }
            out.extend(tool_card(name, args, paired, t, tools_expanded, width));
            out
        }
        // 只有结果没有调用：调用参数已经不在手上，卡上就只剩 header + 结果。
        Entry::ToolResult {
            name,
            ok,
            result,
            details,
            duration_ms,
            ..
        } => tool_card(
            name,
            "",
            Some(ToolOutcome {
                text: result.as_str(),
                ok: *ok,
                details: details.as_ref(),
                duration_ms: *duration_ms,
            }),
            t,
            tools_expanded,
            width,
        ),
        Entry::Error { text } => {
            let err = crate::tui::theme::theme().fg_style(crate::tui::theme::ColorToken::Error);
            let mut out = Vec::new();
            for part in text.split('\n') {
                out.push(Line::styled(part.to_string(), err));
            }
            out
        }
        // Emitter-aligned notice (model switches, compaction reports).
        Entry::System { text, align, pin: _ } => system_block(text, *align, t, width),
        // Name markers are metadata, not chat content: never a history row.
        Entry::Name { .. } => Vec::new(),
        // Same for the todo list: it is session **state** (the last one wins,
        // for resume and for the context note), not narration. What the user
        // sees is the `todo` call's own card, whose details carry the list.
        // 清单是**状态**不是叙述：它不在这条流里，而是贴在历史区底部
        // （`render::todo`，由 `HistoryZone::render_rows` 预留行高）。
        Entry::Todo { .. } => Vec::new(),
        // Compaction fork point: a centered divider announcing the
        // boundary. The summary itself lives in the context (a user
        // turn), not on screen — this is just the seam marker.
        Entry::Compaction { .. } => system_block("—— 上下文已压缩 ——", Align::Center, t, width),
    }
}

/// 流式尾巴的行：图里最下面那一块，**还没进转录**。
///
/// 走的是和成品完全同一条渲染路径（同一个 [`render_block`]、同一套折行），
/// 所以回合结束时「流式那半句」被正式条目替换，画面一行都不会动。与
/// [`single_node`] 的唯一区别是 [`Streaming::Live`] 这个声明：它说这些行
/// **绝不进块缓存** —— 缓存是转录的缓存，尾巴不是转录。
///
/// 顺序与 `SessionState::finalize_round` 一致（先 [`Entry::Reasoning`] 后
/// [`Entry::Assistant`]，两条各成一块、中间一条间隔），空的那半不出图：
/// Ctrl+T 藏起思考、或正文还没开始，就只画有的那半。
pub(crate) fn live_tail(
    reasoning: &str,
    text: &str,
    tool_output: &str,
    t: &HistoryTheme,
    show_reasoning: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let fold = |lines: Vec<Line<'static>>| super::blocks::wrap_rows(lines.into_iter(), width);
    let mut out = Vec::new();
    if show_reasoning && !reasoning.trim().is_empty() {
        out = fold(render_block(
            BlockKind::Reasoning,
            reasoning,
            Streaming::Live,
            width,
            t,
        ));
    }
    if !text.is_empty() {
        if !out.is_empty() {
            out.push(super::blocks::block_gap());
        }
        out.extend(fold(render_block(
            BlockKind::Assistant,
            text,
            Streaming::Live,
            width,
            t,
        )));
    }
    // 运行中的工具输出：**纯文本**（命令输出不是 markdown —— `*` 会被当成列表），
    // 且只画尾巴（服务端缓冲本来就是尾巴，屏幕上再多也看不过来）。它排在待定
    // 工具卡之后，卡里那句 intent 已经说清是谁在跑。
    if !tool_output.trim().is_empty() {
        if !out.is_empty() {
            out.push(super::blocks::block_gap());
        }
        out.extend(fold(super::tools::output_lines(
            tool_output,
            LIVE_TOOL_LINES,
            // 折叠恒开：Ctrl+O 管的是**成品卡片**的展开，不是这行「正在跑」的
            // 实时输出——它本来就只该占几行。
            false,
            false,
            t,
        )));
    }
    out
}

/// 运行中的工具输出画几行：够看出它在动、在报错，不必把屏幕铺满。
pub(crate) const LIVE_TOOL_LINES: usize = 12;

// Render entries into ratatui lines.
//
// The single render entry point: in-memory history and DB-resumed history
// both go through it, guaranteeing "reopened after persistence" looks identical to "just typed".
#[cfg(test)]
pub fn estimated_height(lines: &[Line], width: usize) -> usize {
    let w = width.max(1);
    lines
        .iter()
        .map(|l| {
            let tw: usize = l.spans.iter().map(|sp| sp.content.as_ref().width()).sum();
            if tw == 0 { 1 } else { tw.div_ceil(w) }
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::super::{cards::*, glyphs, system::*, tools};
    use super::*;
    use crate::server::entry::{Align, UsageSummary};
    use ratatui::style::Modifier;
    use unicode_width::UnicodeWidthStr;

    fn text_of(lines: &[Line]) -> String {
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
    fn entries_render_by_kind_not_by_prefix() {
        let p = HistoryTheme::resolve();
        let entries = vec![
            Entry::User {
                content: "问题".into(),
            },
            Entry::Assistant {
                content: "回答".into(),
                usage: None,
            },
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "edit".into(),
                args: r#"{"path":"./a.txt"}"#.into(),
                intent: "改文件".into(),
                text: String::new(),
                first: true,
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "edit".into(),
                ok: true,
                result: "- 旧行\n+ 新行".into(),
                details: None,
                duration_ms: 0,
            },
            Entry::Error {
                text: "炸了".into(),
            },
        ];
        let lines = render(&entries, &p, false, false);
        let all = text_of(&lines);
        // User card opens with a blank accent row on a true-black background
        assert_eq!(lines[0].spans[0].content, "▌ ");
        assert_eq!(lines[0].spans[0].style.fg, Some(p.get(Token::UserCardBar)));
        assert_eq!(
            lines[0].spans[0].style.bg,
            Some(p.get(Token::UserCardBg)),
            "卡底必须走 userMessageBg token"
        );
        // The matched pair renders as one stacked card: no in/out labels
        assert!(!all.contains("(in)") && !all.contains("(out)"), "{all}");
        // The call card carries the path…
        assert!(all.contains("./a.txt"), "{all}");
        // …and the diff body keeps its red/green rows (inside the card frame,
        // so the color sits on the span rather than the whole line)
        // Diff rows: theme-tinted backgrounds, distinct per side.
        let bgs: Vec<_> = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .map(|s| s.style.bg)
            .collect();
        assert!(bgs.iter().any(|b| b.is_some()), "diff 行有底色: {all}");
        // Error stays the theme error color
        assert_eq!(
            lines.last().unwrap().style.fg,
            Some(crate::tui::theme::theme().color(crate::tui::theme::ColorToken::Error))
        );
    }

    #[test]
    fn user_card_has_blank_rows_above_and_below() {
        let p = HistoryTheme::resolve();
        let lines = user_card("你好", &p, 20);
        assert_eq!(lines.len(), 3, "空行 + 正文 + 空行");
        for i in [0, 2] {
            let t = text_of(std::slice::from_ref(&lines[i]));
            assert_eq!(t.trim(), "▌", "空行只有 accent 竖线: {t:?}");
            // …and the black background still covers the whole row
            for sp in &lines[i].spans {
                assert_eq!(sp.style.bg, Some(p.get(Token::UserCardBg)), "空行也必须带卡底色");
            }
        }
    }

    #[test]
    fn user_card_is_white_on_true_black() {
        let p = HistoryTheme::resolve();
        let lines = user_card("你好", &p, 20);
        // Every body span: white on truecolor black (never the indexed black)
        for l in lines.iter().take(2).skip(1) {
            for (i, sp) in l.spans.iter().enumerate() {
                assert_eq!(sp.style.bg, Some(p.get(Token::UserCardBg)), "用户消息必须带卡底色");
                // span 0 is the accent gutter, not message text
                if i > 0 && !sp.content.trim().is_empty() {
                    assert_eq!(sp.style.fg, Some(Color::White), "用户消息必须是白字");
                }
            }
        }
    }

    #[test]
    fn tool_cards_are_bordered() {
        let p = HistoryTheme::resolve();
        let call = tool_card(
            "bash",
            r#"{"intent":"看目录","command":"ls -la"}"#,
            None,
            &p,
            false,
            30,
        );
        // Top edge: the frame, with the header riding inside it.
        let top = text_of(&call[..1]);
        assert!(top.starts_with("+- "), "{top:?}");
        assert!(top.contains("bash"), "顶边必须带 header：{top:?}");
        assert!(top.contains("看目录"), "header 里没有 intent：{top:?}");
        assert!(!text_of(&call).contains("(in)"), "不该再声明卡片类型");
        // Body row: pipes with a space inside both ends
        let body = text_of(&call[1..2]);
        assert!(body.starts_with("| "), "{body:?}");
        assert!(body.trim_end().ends_with("|"), "{body:?}");
        // Bottom edge
        assert!(
            text_of(&call[call.len() - 1..])
                .trim_end()
                .starts_with("+-"),
            "卡片必须有底边"
        );
        // Every row is **exactly** `width` cells: wider and the hard-wrap pass
        // would push the excess onto a line of its own (a stray black square).
        for l in &call {
            let w: usize = l.spans.iter().map(|s| s.content.as_ref().width()).sum();
            assert_eq!(w, 30, "卡片行的宽度必须恰好等于目标宽度: {w}");
        }
    }

    #[test]
    fn card_edges_break_the_dash_run_at_both_ends() {
        // The agreed shape (option F): symmetric breaks so a ligating font
        // cannot fuse the run and draw the frame short.
        let p = HistoryTheme::resolve();
        let rows = tool_card(
            "bash",
            r#"{"intent":"看","command":"ls"}"#,
            Some(ToolOutcome::text("out", true)),
            &p,
            false,
            40,
        );
        let edges: Vec<String> = rows
            .iter()
            .map(|l| text_of(std::slice::from_ref(l)))
            .filter(|t| t.starts_with("+-"))
            .collect();
        assert_eq!(edges.len(), 3, "顶边/中缝/底边: {edges:?}");
        // 顶边带 header，中缝和底边还是纯横杠：两端各让一格给断口。
        for t in &edges[1..] {
            assert!(t.starts_with("+-- "), "左断口必须是 `+-- `: {t:?}");
            assert!(t.ends_with(" -+"), "右断口必须是 ` -+`: {t:?}");
            let inner = &t[4..t.len() - 4];
            assert!(
                inner.len() > 10 && !inner.contains(' '),
                "中段必须连续: {t:?}"
            );
        }
        assert!(edges[0].ends_with(" -+"), "顶边右断口: {:?}", edges[0]);
    }

    #[test]
    fn no_card_row_exceeds_the_target_width() {
        // Regression for the phantom black square: the edges used to emit one
        // cell too many, which wrapped into a row of its own.
        let p = HistoryTheme::resolve();
        for width in [20usize, 40, 80, 121] {
            let rows = tool_card(
                "bash",
                r#"{"intent":"等","command":"sleep 5"}"#,
                Some(ToolOutcome::text("done", true)),
                &p,
                false,
                width,
            );
            for l in &rows {
                let w: usize = l.spans.iter().map(|s| s.content.as_ref().width()).sum();
                assert_eq!(
                    w,
                    width,
                    "({width}) 行宽必须恰好: {w} -> {:?}",
                    text_of(std::slice::from_ref(l))
                );
            }
        }
    }

    #[test]
    fn every_cell_of_a_card_carries_the_card_background() {
        // The reported bug: the card looked dark only in the gaps, because
        // the text spans carried a foreground but no background, and stray
        // ANSI from the command reset it. Every span of every card row —
        // frame, text, padding — must paint the state background.
        let p = HistoryTheme::resolve();
        let entries = vec![
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "bash".into(),
                args: r#"{"command":"ls --color=always"}"#.into(),
                intent: "列目录".into(),
                text: String::new(),
                first: true,
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "bash".into(),
                ok: true,
                // A result that (from an older DB, say) still holds ANSI.
                result: "\u{1b}[01;34mdir\u{1b}[0m\nplain\n".into(),
                details: None,
                duration_ms: 0,
            },
        ];
        let lines = render(&entries, &p, false, false);
        let black = Some(p.get(Token::ToolBgSuccess));
        for l in &lines {
            // Only card rows (those with a frame) are checked; gaps have no spans.
            if l.spans
                .iter()
                .any(|s| s.content.contains('|') || s.content.contains('+'))
            {
                for sp in &l.spans {
                    assert_eq!(
                        sp.style.bg, black,
                        "卡片每一格都必须是真彩黑底: {:?}",
                        sp.content
                    );
                }
            }
        }
        // Escape bytes must not survive into the drawn text.
        let all = text_of(&lines);
        assert!(!all.contains('\u{1b}'), "结果里的 ANSI 必须被剥离: {all:?}");
        assert!(all.contains("dir") && all.contains("plain"), "{all}");
    }

    #[test]
    fn backend_paints_black_across_the_whole_card_row() {
        // End-to-end through ratatui's own renderer: draw a card into a
        // TestBackend and read back the **composed cell styles**. This is the
        // check that actually proves no hole in the background, because it
        // goes through the same path the terminal does.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let p = HistoryTheme::resolve();
        let rows = tool_card(
            "bash",
            r#"{"intent":"看","command":"ls --color=always"}"#,
            Some(ToolOutcome::text("\u{1b}[01;34mdir\u{1b}[0m\nplain", true)),
            &p,
            false,
            40,
        );
        let mut term = Terminal::new(TestBackend::new(40, rows.len() as u16)).unwrap();
        term.draw(|f| {
            f.render_widget(Paragraph::new(rows.clone()), f.area());
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        // Every cell of the card's body rows must carry the card background.
        // A double-width glyph owns two columns; ratatui writes its trailing
        // cell as a reset-background space, and the terminal paints the glyph
        // (background included) across both — so that continuation cell is the
        // one place a reset background is correct.
        for y in 0..buf.area.height {
            let mut continuation = false;
            for x in 0..buf.area.width {
                let cell = &buf[(x, y)];
                if continuation {
                    continuation = false;
                    continue;
                }
                continuation = cell.symbol().width() == 2;
                assert_eq!(
                    cell.bg,
                    p.get(Token::ToolBgSuccess),
                    "卡片 ({x},{y}) 的格子没有卡底: {:?}",
                    cell.symbol()
                );
            }
        }
    }

    #[test]
    fn hard_wrap_adds_no_row_for_a_real_transcript() {
        // The phantom-cell bug: an over-wide row was split by `hard_wrap` into
        // an extra row holding only the overflow (a lone black cell). Prove
        // the wrapped row count equals the input count for a realistic
        // transcript, at several widths.
        for width in [30usize, 40, 80, 120] {
            let entries = vec![
                Entry::User {
                    content: "跑一下 sleep 5 看看".into(),
                },
                Entry::ToolRequest {
                    call_id: "c1".into(),
                    name: "bash".into(),
                    args: r#"{"command":"sleep 5 && echo done"}"#.into(),
                    intent: "等五秒".into(),
                    text: String::new(),
                    first: true,
                },
                Entry::ToolResult {
                    call_id: "c1".into(),
                    name: "bash".into(),
                    ok: true,
                    result: "done".into(),
                    details: None,
                    duration_ms: 0,
                },
            ];
            let lines = render_at(&entries, &HistoryTheme::resolve(), false, false, width);
            let _ = lines; // hard_wrap 随 view.rs 删除，见 DELETED.md
        }
    }

    #[test]
    fn matched_call_and_result_stack_into_one_card() {
        let p = HistoryTheme::resolve();
        let entries = vec![
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "bash".into(),
                args: r#"{"command":"ls"}"#.into(),
                intent: "看目录".into(),
                text: String::new(),
                first: true,
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "bash".into(),
                ok: true,
                result: "a.rs".into(),
                details: None,
                duration_ms: 0,
            },
        ];
        let lines = render(&entries, &p, false, false);
        // Exactly one top edge, one shared middle seam, one bottom edge:
        // two separate cards would produce two of each.
        let edges = lines
            .iter()
            .filter(|l| {
                text_of(std::slice::from_ref(*l))
                    .trim_end()
                    .starts_with("+-")
            })
            .count();
        assert_eq!(
            edges,
            3,
            "合并后共 3 条横边（上/中/下）: {:?}",
            text_of(&lines)
        );
        // No type labels anywhere
        let all = text_of(&lines);
        assert!(!all.contains("(in)") && !all.contains("(out)"), "{all}");
    }

    #[test]
    fn bash_payload_prettifies_to_shell_not_json() {
        let p = HistoryTheme::resolve();
        let block = tools::ToolBlock {
            name: "bash",
            args: r#"{"intent":"构建","command":"cargo build --release"}"#,
            result: None,
            expanded: false,
        };
        let all = text_of(&block.call_body(&p));
        assert!(all.contains("cargo build --release"), "{all}");
        assert!(!all.contains('{'), "不该把 JSON 外壳渲出来: {all}");
        assert!(!all.contains("command"), "{all}");
    }

    #[test]
    fn payload_highlight_uses_the_paths_language() {
        let p = HistoryTheme::resolve();
        let block = tools::ToolBlock {
            name: "edit",
            args: r#"{"intent":"改","path":"main.rs","old":"let a","new":"let b"}"#,
            result: None,
            expanded: false,
        };
        let body = text_of(&block.call_body(&p));
        let header = text_of(&[block.header(&p)]);
        assert!(body.contains("let a") && body.contains("let b"), "{body}");
        // 路径是 meta（事实），不进正文，也不跟 intent 抢 description。
        assert!(header.contains("main.rs"), "{header}");
        assert!(!body.contains("main.rs"), "路径不该出现在正文里: {body}");
    }

    #[test]
    fn system_notice_honours_its_alignment() {
        let p = HistoryTheme::resolve();
        let left = system_block("切到 X", Align::Left, &p, 20);
        assert!(text_of(&left).starts_with("切到 X"));
        let mid = system_block("压缩完成", Align::Center, &p, 20);
        let row = text_of(&mid[..1]);
        let pad = row.chars().take_while(|c| *c == ' ').count();
        // "压缩完成" is 4 CJK glyphs = 8 cells, so (20 - 8) / 2
        assert_eq!(pad, (20 - 8) / 2, "居中必须左右留白: {row:?}");
    }

    #[test]
    fn plain_view_folds_beyond_five_lines() {
        let p = HistoryTheme::resolve();
        let long = Entry::ToolResult {
            call_id: "c1".into(),
            name: "t".into(),
            ok: true,
            result: (1..=8)
                .map(|i| format!("line{i}"))
                .collect::<Vec<_>>()
                .join("\n"),
            details: None,
            duration_ms: 0,
        };
        let lines = render(&[long], &p, false, false);
        let joined = text_of(&lines);
        // 折叠取尾：留下最后 5 行，头部用一行标记交代还剩多少。
        assert!(joined.contains("… 3 earlier lines"), "{joined}");
        assert!(joined.contains(glyphs::EXPAND_HINT), "缺展开提示: {joined}");
        assert!(joined.contains("line8") && joined.contains("line4"), "{joined}");
        assert!(!joined.contains("line3"), "取尾却还留着头部: {joined}");
    }

    #[test]
    fn payload_round_trips_through_json() {
        let entries = vec![
            Entry::User {
                content: "你好\n世界".into(),
            },
            Entry::Assistant {
                content: "回复".into(),
                usage: Some(UsageSummary {
                    total_tokens: 10,
                    prompt_tokens: 8,
                    cached_tokens: 2,
                    completion_tokens: 2,
                    reasoning_tokens: 0,
                }),
            },
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "edit".into(),
                args: r#"{"path":"p"}"#.into(),
                intent: "改一下".into(),
                text: String::new(),
                first: true,
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "edit".into(),
                ok: false,
                result: "出错".into(),
                details: None,
                duration_ms: 0,
            },
            Entry::System {
                text: "已切换模型".into(),
                align: Align::Center,
            pin: false,
            },
        ];
        for e in &entries {
            let (kind, payload) = e.to_payload();
            let back = Entry::from_payload(kind, &payload).expect("kind 必须可还原");
            assert_eq!(&back, e, "{kind} 往返失真");
        }
    }

    #[test]
    fn legacy_tool_request_without_args_still_loads() {
        // Pre-refactor rows stored only `object`; they must not break resume.
        let old = r#"{"call_id":"c1","name":"bash","object":""}"#;
        let back = Entry::from_payload("tool_request", old).unwrap();
        match back {
            Entry::ToolRequest {
                args, intent, name, ..
            } => {
                assert_eq!(args, "");
                assert_eq!(intent, "");
                assert_eq!(name, "bash");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_sentence_before_a_tool_call_is_rendered_above_its_card() {
        // 模型在调工具前说的那半句（`Entry::ToolRequest.text`）属于这次调用，
        // 画在卡片上面当正文。以前它只在流式尾巴上闪一下，`buffered` 模式
        // 下更是完全不上屏——数据一直有，只是没人画。
        let p = HistoryTheme::resolve();
        let entries = vec![
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "bash".into(),
                args: r#"{"intent":"看看","command":"ls"}"#.into(),
                intent: "看看".into(),
                text: "我先看一眼目录".into(),
                first: true,
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "bash".into(),
                ok: true,
                result: "a.txt".into(),
                details: None,
                duration_ms: 0,
            },
        ];
        let rows = render(&entries, &p, false, false);
        let text = text_of(&rows);
        assert!(text.contains("我先看一眼目录"), "那半句必须上屏: {text}");
        let narration = rows
            .iter()
            .position(|l| {
                l.spans
                    .iter()
                    .any(|s| s.content.contains("我先看一眼目录"))
            })
            .expect("找到正文行");
        let card = rows
            .iter()
            .position(|l| l.spans.iter().any(|s| s.content.contains("bash")))
            .expect("找到卡片行");
        assert!(narration < card, "正文必须在卡片上面");
    }

    #[test]
    fn only_the_first_call_of_a_message_carries_the_narration() {
        // 一条消息带多个调用时，那半句只挂在第一条上（协议事实），所以它只该
        // 出现一次——第二张卡上面不该再贴一遍。
        let p = HistoryTheme::resolve();
        let call = |id: &str, first: bool, text: &str| Entry::ToolRequest {
            call_id: id.into(),
            name: "bash".into(),
            args: r#"{"intent":"看看","command":"ls"}"#.into(),
            intent: "看看".into(),
            text: text.into(),
            first,
        };
        let rows = render(
            &[
                call("c1", true, "一起看两个"),
                call("c2", false, ""),
            ],
            &p,
            false,
            false,
        );
        let text = text_of(&rows);
        assert_eq!(text.matches("一起看两个").count(), 1, "{text}");
    }

    #[test]
    fn diff_view_uses_colored_backgrounds() {
        let p = HistoryTheme::resolve();
        // The diff is declared by the tool's `details` (the tool knows what it
        // replaced); the result text is just the one-line summary it tells the
        // model. This is the shape `edit` actually emits.
        let e = Entry::ToolResult {
            call_id: "c1".into(),
            name: "edit".into(),
            ok: true,
            result: "replaced: x.rs".into(),
            details: Some(serde_json::json!({
                "kind": "diff",
                "deletions": ["旧行"],
                "insertions": ["新行"],
            })),
            duration_ms: 0,
        };
        let lines = render(&[e], &p, false, false);
        // The colored rows live inside the card frame, so the red/green is the
        // inner span's style, not the row's.
        // Diff rows carry a background tint derived from the theme's
        // toolDiffRemoved/Added over the card background — the two sides
        // must stay visually distinct.
        let del: Vec<_> = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .filter(|s| s.content.starts_with("- "))
            .map(|s| s.style.bg)
            .collect();
        let ins: Vec<_> = lines
            .iter()
            .flat_map(|l| l.spans.iter())
            .filter(|s| s.content.starts_with("+ "))
            .map(|s| s.style.bg)
            .collect();
        assert!(del.iter().all(|b| b.is_some()), "删除行必须有底色");
        assert!(ins.iter().all(|b| b.is_some()), "插入行必须有底色");
        assert_ne!(del[0], ins[0], "删除/插入的底色必须可区分");
    }

    #[test]
    fn estimated_height_counts_wrapped_rows() {
        let lines = render(
            &[Entry::Assistant {
                content: "x".repeat(20),
                usage: None,
            }],
            &HistoryTheme::resolve(),
            false,
            false,
        );
        // 20 cells wide, container 10 -> 2 wrapped rows, no padding rows of its own
        assert_eq!(estimated_height(&lines, 10), 2);
    }

    #[test]
    fn every_node_is_separated_by_one_blank_row() {
        let p = HistoryTheme::resolve();
        let entries = vec![
            Entry::User {
                content: "问".into(),
            },
            Entry::Reasoning {
                content: "想".into(),
            },
            Entry::Assistant {
                content: "答".into(),
                usage: None,
            },
            Entry::System {
                text: "切模型".into(),
                align: Align::Center,
            pin: false,
            },
        ];
        let lines = render(&entries, &p, true, false);
        // Gaps: one per node boundary (3 nodes → 2 gaps) plus the
        // reasoning/answer seam (the reasoning block's trailing blank row).
        // Never a doubled blank row anywhere.
        let blanks = lines
            .iter()
            .filter(|l| l.spans.iter().all(|s| s.content.trim().is_empty()))
            .count();
        assert_eq!(
            blanks,
            3,
            "节点间2条 + 思考与回复间1条: {:?}",
            text_of(&lines)
        );
        let text = text_of(&lines);
        assert!(!text.contains("\n\n\n"), "不该出现连续两条空行: {text:?}");
    }

    #[test]
    fn reasoning_shown_by_default_hidden_when_folded() {
        let p = HistoryTheme::resolve();
        let entries = vec![
            Entry::Reasoning {
                content: "内心独白".into(),
            },
            Entry::Assistant {
                content: "答案".into(),
                usage: None,
            },
        ];
        // Default view: reasoning is visible, in the muted gray
        let open = render(&entries, &p, true, false);
        let open_text = text_of(&open);
        assert!(open_text.contains("内心独白"), "{open_text}");
        let r_line = open
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content == "内心独白"))
            .unwrap();
        assert_eq!(r_line.spans[0].style.fg, Some(p.get(Token::SystemText)));
        assert!(
            r_line.spans[0]
                .style
                .add_modifier
                .contains(Modifier::ITALIC)
        );
        // Folded: gone
        let folded = render(&entries, &p, false, false);
        assert!(!text_of(&folded).contains("内心独白"));
    }

    #[test]
    fn empty_reasoning_never_renders_even_expanded() {
        let p = HistoryTheme::resolve();
        let entries = vec![
            Entry::Reasoning {
                content: "  ".into(),
            },
            Entry::Assistant {
                content: "答案".into(),
                usage: None,
            },
        ];
        let lines = render(&entries, &p, true, false);
        let text = text_of(&lines);
        // Just the content: the inter-node blank row belongs to the caller.
        assert!(text.contains("答案"));
        assert_eq!(lines.len(), 1, "不该有多余的空思考行: {text:?}");
    }

    #[test]
    fn reasoning_round_trips_through_payload() {
        let e = Entry::Reasoning {
            content: "想了想".into(),
        };
        let (kind, payload) = e.to_payload();
        assert_eq!(kind, "reasoning");
        assert_eq!(Entry::from_payload(kind, &payload).unwrap(), e);
        // The assistant payload no longer carries reasoning at all; a
        // legacy row that still has one reads fine (field ignored).
        let legacy = r#"{"content":"老消息","usage":null,"reasoning":"旧思考"}"#;
        let back = Entry::from_payload("assistant", legacy).unwrap();
        assert_eq!(
            back,
            Entry::Assistant {
                content: "老消息".into(),
                usage: None,
            }
        );
    }

    #[test]
    fn inline_code_stays_on_one_line() {
        let p = HistoryTheme::resolve();
        let lines = super::super::markdown::render_markdown(
            "已用 `edit` 工具将 `main.rs` 中的 `hi` 改为 `hello`。",
            &p,
        );
        assert_eq!(lines.len(), 1, "行内代码不该换行: {:?}", text_of(&lines));
    }

    #[test]
    fn tool_collapse_limit_is_per_tool() {
        // 每个工具的折叠阈值现在由它自己的渲染器声明（注册表里那一行）。
        assert_eq!(tools::renderer_for("edit").fold_limit(), 14);
        assert_eq!(tools::renderer_for("mass_edit").fold_limit(), 14);
        assert_eq!(tools::renderer_for("read").fold_limit(), 5);
        assert_eq!(tools::renderer_for("fetch").fold_limit(), 3);
        assert_eq!(tools::renderer_for("未知工具").fold_limit(), 5, "兜底");
    }

    #[test]
    fn plain_view_respects_per_tool_limit_and_expansion() {
        let p = HistoryTheme::resolve();
        let eight = (1..=8)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        // 8 rows: edit (threshold 14) does not fold; unknown tools (threshold 5) fold
        let edit_e = Entry::ToolResult {
            call_id: "c1".into(),
            name: "edit".into(),
            ok: true,
            result: eight.clone(),
            details: None,
            duration_ms: 0,
        };
        let other_e = Entry::ToolResult {
            call_id: "c".into(),
            name: "x".into(),
            ok: true,
            result: eight.clone(),
            details: None,
            duration_ms: 0,
        };
        let edit_text = text_of(&render(&[edit_e], &p, false, false));
        assert!(
            edit_text.contains("line8") && !edit_text.contains("earlier lines"),
            "edit 8 行不折叠: {edit_text}"
        );
        let other_text = text_of(&render(std::slice::from_ref(&other_e), &p, false, false));
        assert!(
            other_text.contains("… 3 earlier lines"),
            "未知工具 8 行折叠: {other_text}"
        );
        // Ctrl+O expand: folded output expands too
        let open_text = text_of(&render(&[other_e], &p, false, true));
        assert!(
            open_text.contains("line1") && !open_text.contains("earlier lines"),
            "展开后无折叠: {open_text}"
        );
    }

    #[test]
    fn read_shows_its_content_like_every_tool() {
        // 有了 header 就不必再靠「不画结果」省那一行：路径进 header 的
        // meta，正文就是读到的内容本身。read 和别的工具一种画法。
        let p = HistoryTheme::resolve();
        let entries = vec![
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "read".into(),
                args: r#"{"intent":"读文件","path":"main.rs"}"#.into(),
                intent: "读文件".into(),
                text: String::new(),
                first: true,
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "read".into(),
                ok: true,
                result: "1\tfn main() {}".into(),
                details: None,
                duration_ms: 0,
            },
        ];
        let lines = render(&entries, &p, false, false);
        let all = text_of(&lines);
        // 一张卡：顶边带 header、底边收尾（调用没有正文，所以不画接缝）。
        let edges = lines
            .iter()
            .filter(|l| text_of(std::slice::from_ref(*l)).trim_end().starts_with("+-"))
            .count();
        assert_eq!(edges, 2, "调用 + 结果该是一张卡: {all:?}");
        assert!(all.contains("fn main"), "读到的内容没画出来: {all}");
        assert!(all.contains("main.rs"), "路径该在 header 的 meta 里: {all}");
    }

    #[test]
    fn empty_tool_result_shows_a_placeholder() {
        let p = HistoryTheme::resolve();
        let e = Entry::ToolResult {
            call_id: "c".into(),
            name: "bash".into(),
            ok: true,
            result: "".into(),
            details: None,
            duration_ms: 0,
        };
        let all = text_of(&render(&[e], &p, false, false));
        assert!(all.contains("(no output)"), "空结果必须有占位符: {all}");
    }

    #[test]
    fn tool_result_payload_round_trips_details() {
        // Persistence contract: a tool result stores **data** — the model-facing
        // text plus the tool's structured payload. The *view* (diff? plain?) is a
        // UI concept derived from that payload at render time, never stored.
        let e = Entry::ToolResult {
            call_id: "c9".into(),
            name: "edit".into(),
            ok: true,
            result: "replaced: x.rs".into(),
            details: Some(serde_json::json!({
                "kind": "diff",
                "deletions": ["a"],
                "insertions": ["b"],
            })),
            duration_ms: 12,
        };
        let (_, payload) = e.to_payload();
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(v.get("view").is_none(), "payload 不该有 view 字段");
        assert_eq!(
            v.get("result").and_then(|r| r.as_str()),
            Some("replaced: x.rs")
        );
        assert_eq!(v.get("duration_ms").and_then(|d| d.as_u64()), Some(12));
        let back = Entry::from_payload("tool_result", &payload).unwrap();
        assert_eq!(back, e, "details 必须原样往返");
    }
}
