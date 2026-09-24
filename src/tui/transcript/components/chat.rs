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
//! Presentation rules (each kind owns its own look):
//! - **user** — a card: true-black background, white text, accent gutter;
//! - **assistant** — plain foreground, no background; reasoning in the muted gray;
//! - **tool** — two bordered cards (`+- … -+`), the call rendered above the result,
//!   both on the same true-black background, payload syntax-highlighted;
//! - **system** — a notice the emitter aligns (left or centered), e.g. compaction
//!   reports.

#[cfg(test)]
use ratatui::style::Color;
use ratatui::text::Line;
#[cfg(test)]
use ratatui::widgets::Paragraph;
#[cfg(test)]
use unicode_width::UnicodeWidthStr;

use super::assistant::assistant_block;
use super::cards::{result_card_visible, tool_exchange, tool_request_card, tool_result_card};
use super::system::system_block;
use crate::entry::Entry;
use crate::tui::theme::Palette;

// Streaming tail: in-flight content at full weight (the final answer),
// re-rendered per delta — the one live piece of the transcript.
pub(crate) fn render_streaming(t: &str, p: &Palette) -> Vec<Line<'static>> {
    crate::tui::components::markdown::render_markdown(t, p)
}

// Render entries into ratatui lines.
//
// The single render entry point: in-memory history and DB-resumed history
// both go through it, guaranteeing "reopened after persistence" looks identical to "just typed".
#[cfg(test)]
pub fn render(
    entries: &[Entry],
    p: &Palette,
    show_reasoning: bool,
    tools_expanded: bool,
) -> Vec<Line<'static>> {
    render_at(entries, p, show_reasoning, tools_expanded, 80)
}

#[cfg(test)]
pub fn render_at(
    entries: &[Entry],
    p: &Palette,
    show_reasoning: bool,
    tools_expanded: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for r in super::super::blocks::blocks(entries) {
        if !out.is_empty() {
            out.push(Line::from(""));
        }
        out.extend(single_node(
            &entries[r.start..r.end],
            p,
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
    p: &Palette,
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
        Entry::User { content } => super::cards::user_card(content, p, width),
        Entry::Assistant {
            content,
            usage,
            reasoning,
        } => assistant_block(
            content,
            reasoning.as_deref(),
            usage.as_ref(),
            p,
            show_reasoning,
        ),
        Entry::ToolRequest { name, args, .. } => {
            if group.len() == 2 {
                let Entry::ToolResult {
                    name: rname,
                    ok,
                    result,
                    ..
                } = &group[1]
                else {
                    unreachable!("group of 2 is always request+result (blocks guarantees)");
                };
                let _ = rname;
                if result_card_visible(name) {
                    tool_exchange(name, args, *ok, result, p, tools_expanded, width)
                } else {
                    tool_request_card(name, args, p, width)
                }
            } else {
                tool_request_card(name, args, p, width)
            }
        }
        Entry::ToolResult {
            name, ok, result, ..
        } => tool_result_card(name, *ok, result, p, tools_expanded, width),
        Entry::Error { text } => {
            let err = crate::tui::theme::theme().fg_style(crate::tui::theme::ColorToken::Error);
            let mut out = Vec::new();
            for part in text.split('\n') {
                out.push(Line::styled(part.to_string(), err));
            }
            out
        }
        // Emitter-aligned notice (model switches, compaction reports).
        Entry::System { text, align } => system_block(text, *align, p, width),
        // Name markers are metadata, not chat content: never a history row.
        Entry::Name { .. } => Vec::new(),
    }
}

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
    use super::super::{cards::*, system::*};
    use super::*;
    use crate::entry::{Align, ToolView, UsageSummary};
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
        let p = Palette::default();
        let entries = vec![
            Entry::User {
                content: "问题".into(),
            },
            Entry::Assistant {
                content: "回答".into(),
                usage: None,
                reasoning: None,
            },
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "edit".into(),
                args: r#"{"path":"./a.txt"}"#.into(),
                intent: "改文件".into(),
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "edit".into(),
                ok: true,
                result: "- 旧行\n+ 新行".into(),
            },
            Entry::Error {
                text: "炸了".into(),
            },
        ];
        let lines = render(&entries, &p, false, false);
        let all = text_of(&lines);
        // User card opens with a blank accent row on a true-black background
        assert_eq!(lines[0].spans[0].content, "▌ ");
        assert_eq!(lines[0].spans[0].style.fg, Some(p.accent));
        assert_eq!(
            lines[0].spans[0].style.bg,
            Some(p.black),
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
        let p = Palette::default();
        let lines = user_card("你好", &p, 20);
        assert_eq!(lines.len(), 3, "空行 + 正文 + 空行");
        for i in [0, 2] {
            let t = text_of(std::slice::from_ref(&lines[i]));
            assert_eq!(t.trim(), "▌", "空行只有 accent 竖线: {t:?}");
            // …and the black background still covers the whole row
            for sp in &lines[i].spans {
                assert_eq!(sp.style.bg, Some(p.black), "空行也必须带卡底色");
            }
        }
    }

    #[test]
    fn user_card_is_white_on_true_black() {
        let p = Palette::default();
        let lines = user_card("你好", &p, 20);
        // Every body span: white on truecolor black (never the indexed black)
        for l in lines.iter().take(2).skip(1) {
            for (i, sp) in l.spans.iter().enumerate() {
                assert_eq!(sp.style.bg, Some(p.black), "用户消息必须带卡底色");
                // span 0 is the accent gutter, not message text
                if i > 0 && !sp.content.trim().is_empty() {
                    assert_eq!(sp.style.fg, Some(Color::White), "用户消息必须是白字");
                }
            }
        }
    }

    #[test]
    fn tool_cards_are_bordered() {
        let p = Palette::default();
        let call = tool_request_card("bash", r#"{"command":"ls -la"}"#, &p, 30);
        // Top edge: bare frame, no tool name / in-out labels
        assert!(
            text_of(&call[..1]).starts_with("+-"),
            "{}",
            text_of(&call[..1])
        );
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
        let p = Palette::default();
        let rows = tool_exchange("bash", r#"{"command":"ls"}"#, true, "out", &p, false, 40);
        let edges: Vec<String> = rows
            .iter()
            .map(|l| text_of(std::slice::from_ref(l)))
            .filter(|t| t.starts_with("+-"))
            .collect();
        assert_eq!(edges.len(), 3, "顶边/中缝/底边: {edges:?}");
        for t in &edges {
            // Confirmed shape: `+-- <one long run> -+`. Each end gives up one
            // cell to a break (the left shows two dashes because the corner
            // itself is a dash).
            assert!(t.starts_with("+-- "), "左断口必须是 `+-- `: {t:?}");
            assert!(t.ends_with(" -+"), "右断口必须是 ` -+`: {t:?}");
            // …and the middle is still one continuous stretch.
            let inner = &t[4..t.len() - 4];
            assert!(
                inner.len() > 10 && !inner.contains(' '),
                "中段必须连续: {t:?}"
            );
        }
    }

    #[test]
    fn no_card_row_exceeds_the_target_width() {
        // Regression for the phantom black square: the edges used to emit one
        // cell too many, which wrapped into a row of its own.
        let p = Palette::default();
        for width in [20usize, 40, 80, 121] {
            let rows = tool_exchange(
                "bash",
                r#"{"command":"sleep 5"}"#,
                true,
                "done",
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
    fn every_cell_of_a_card_carries_the_black_background() {
        // The reported bug: the card looked black only in the gaps, because
        // the text spans carried a foreground but no background, and stray
        // ANSI from the command reset it. Every span of every card row —
        // frame, text, padding — must paint the truecolor black.
        let p = Palette::default();
        let entries = vec![
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "bash".into(),
                args: r#"{"command":"ls --color=always"}"#.into(),
                intent: "列目录".into(),
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "bash".into(),
                ok: true,
                // A result that (from an older DB, say) still holds ANSI.
                result: "\u{1b}[01;34mdir\u{1b}[0m\nplain\n".into(),
            },
        ];
        let lines = render(&entries, &p, false, false);
        let black = Some(p.black);
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

        let p = Palette::default();
        let rows = tool_exchange(
            "bash",
            r#"{"command":"ls --color=always"}"#,
            true,
            "\u{1b}[01;34mdir\u{1b}[0m\nplain",
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
        // Every cell of the card's body rows must be black-backed.
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                let cell = &buf[(x, y)];
                assert_eq!(
                    cell.bg,
                    p.black,
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
                },
                Entry::ToolResult {
                    call_id: "c1".into(),
                    name: "bash".into(),
                    ok: true,
                    result: "done".into(),
                },
            ];
            let lines = render_at(&entries, &Palette::default(), false, false, width);
            let before = lines.len();
            let wrapped = crate::tui::view::hard_wrap_for_test(&lines, width);
            assert_eq!(
                wrapped.len(),
                before,
                "({width}) 不该有任何行被硬换行切出额外的行"
            );
            // And the blank separator rows must be genuinely blank (no stray
            // black cell): no spans at all.
            for l in &wrapped {
                let text: String = l.spans.iter().map(|s| s.content.to_string()).collect();
                if text.trim().is_empty() {
                    assert!(
                        l.spans.is_empty(),
                        "({width}) 空行不该带任何样式格子: {text:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn matched_call_and_result_stack_into_one_card() {
        let p = Palette::default();
        let entries = vec![
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "bash".into(),
                args: r#"{"command":"ls"}"#.into(),
                intent: "看目录".into(),
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "bash".into(),
                ok: true,
                result: "a.rs".into(),
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
        let p = Palette::default();
        let lines = payload_lines("bash", r#"{"command":"cargo build --release"}"#, &p);
        let all = text_of(&lines);
        assert!(all.contains("cargo build --release"), "{all}");
        assert!(!all.contains('{'), "不该把 JSON 外壳渲出来: {all}");
        assert!(!all.contains("command"), "{all}");
    }

    #[test]
    fn payload_highlight_uses_the_paths_language() {
        let p = Palette::default();
        let lines = payload_lines(
            "edit",
            r#"{"path":"main.rs","old":"let a","new":"let b"}"#,
            &p,
        );
        let all = text_of(&lines);
        assert!(
            all.contains("main.rs") && all.contains("let a") && all.contains("let b"),
            "{all}"
        );
    }

    #[test]
    fn system_notice_honours_its_alignment() {
        let p = Palette::default();
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
        let p = Palette::default();
        let long = Entry::ToolResult {
            call_id: "c1".into(),
            name: "t".into(),
            ok: true,
            result: (1..=8)
                .map(|i| format!("line{i}"))
                .collect::<Vec<_>>()
                .join("\n"),
        };
        let lines = render(&[long], &p, false, false);
        let joined = text_of(&lines);
        assert!(joined.contains("line1") && joined.contains("line5"));
        assert!(!joined.contains("line6"), "{joined}");
        assert!(joined.contains("共 8 行"));
    }

    #[test]
    fn payload_round_trips_through_json() {
        let entries = vec![
            Entry::User {
                content: "你好\n世界".into(),
            },
            Entry::Assistant {
                content: "回复".into(),
                reasoning: None,
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
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "edit".into(),
                ok: false,
                result: "出错".into(),
            },
            Entry::System {
                text: "已切换模型".into(),
                align: Align::Center,
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
    fn diff_view_uses_colored_backgrounds() {
        let p = Palette::default();
        let e = Entry::ToolResult {
            call_id: "c1".into(),
            name: "edit".into(),
            ok: true,
            result: "- 旧行\n+ 新行".into(),
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
                reasoning: None,
            }],
            &Palette::default(),
            false,
            false,
        );
        // 20 cells wide, container 10 -> 2 wrapped rows, no padding rows of its own
        assert_eq!(estimated_height(&lines, 10), 2);
    }

    #[test]
    fn every_node_is_separated_by_one_blank_row() {
        let p = Palette::default();
        let entries = vec![
            Entry::User {
                content: "问".into(),
            },
            Entry::Assistant {
                content: "答".into(),
                usage: None,
                reasoning: Some("想".into()),
            },
            Entry::System {
                text: "切模型".into(),
                align: Align::Center,
            },
        ];
        let lines = render(&entries, &p, true, false);
        // Gaps: node boundaries (2) plus the reasoning/answer seam inside
        // the assistant block (1). Never a doubled blank row anywhere.
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
        let p = Palette::default();
        let entries = vec![Entry::Assistant {
            content: "答案".into(),
            usage: None,
            reasoning: Some("内心独白".into()),
        }];
        // Default view: reasoning is visible, in the muted gray
        let open = render(&entries, &p, true, false);
        let open_text = text_of(&open);
        assert!(open_text.contains("内心独白"), "{open_text}");
        let r_line = open
            .iter()
            .find(|l| l.spans.iter().any(|s| s.content == "内心独白"))
            .unwrap();
        assert_eq!(r_line.spans[0].style.fg, Some(p.muted));
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
        let p = Palette::default();
        let entries = vec![Entry::Assistant {
            content: "答案".into(),
            usage: None,
            reasoning: Some("  ".into()),
        }];
        let lines = render(&entries, &p, true, false);
        let text = text_of(&lines);
        // Just the content: the inter-node blank row belongs to the caller.
        assert!(text.contains("答案"));
        assert_eq!(lines.len(), 1, "不该有多余的空思考行: {text:?}");
    }

    #[test]
    fn reasoning_round_trips_through_payload() {
        let e = Entry::Assistant {
            content: "答".into(),
            usage: None,
            reasoning: Some("想了想".into()),
        };
        let (kind, payload) = e.to_payload();
        assert_eq!(Entry::from_payload(kind, &payload).unwrap(), e);
        // Old data lacks the reasoning field: reads as None without exploding
        let legacy = r#"{"content":"老消息","usage":null}"#;
        let back = Entry::from_payload("assistant", legacy).unwrap();
        assert_eq!(
            back,
            Entry::Assistant {
                content: "老消息".into(),
                usage: None,
                reasoning: None
            }
        );
    }

    #[test]
    fn inline_code_stays_on_one_line() {
        let p = Palette::default();
        let lines = crate::tui::components::markdown::render_markdown(
            "已用 `edit` 工具将 `main.rs` 中的 `hi` 改为 `hello`。",
            &p,
        );
        assert_eq!(lines.len(), 1, "行内代码不该换行: {:?}", text_of(&lines));
    }

    #[test]
    fn tool_collapse_limit_is_per_tool() {
        assert_eq!(collapse_limit("edit"), 14);
        assert_eq!(collapse_limit("mass_edit"), 14);
        assert_eq!(collapse_limit("未知工具"), 5);
    }

    #[test]
    fn plain_view_respects_per_tool_limit_and_expansion() {
        let p = Palette::default();
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
        };
        let other_e = Entry::ToolResult {
            call_id: "c".into(),
            name: "x".into(),
            ok: true,
            result: eight.clone(),
        };
        let edit_text = text_of(&render(&[edit_e], &p, false, false));
        assert!(
            edit_text.contains("line8") && !edit_text.contains("共"),
            "edit 8 行不折叠: {edit_text}"
        );
        let other_text = text_of(&render(std::slice::from_ref(&other_e), &p, false, false));
        assert!(
            other_text.contains("共 8 行"),
            "未知工具 8 行折叠: {other_text}"
        );
        // Ctrl+O expand: folded output expands too
        let open_text = text_of(&render(&[other_e], &p, false, true));
        assert!(
            open_text.contains("line8") && !open_text.contains("共"),
            "展开后无折叠: {open_text}"
        );
    }

    #[test]
    fn read_has_no_result_card() {
        let p = Palette::default();
        let entries = vec![
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "read".into(),
                args: r#"{"path":"main.rs"}"#.into(),
                intent: "读文件".into(),
            },
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "read".into(),
                ok: true,
                result: "1\tfn main() {}".into(),
            },
        ];
        let lines = render(&entries, &p, false, false);
        // Upper card only: one top edge, one bottom edge, no seam.
        let edges = lines
            .iter()
            .filter(|l| {
                text_of(std::slice::from_ref(*l))
                    .trim_end()
                    .starts_with("+-")
            })
            .count();
        assert_eq!(edges, 2, "read 只该有一张卡片: {:?}", text_of(&lines));
        assert!(
            !text_of(&lines).contains("fn main"),
            "读到的内容不该重复出现"
        );
    }

    #[test]
    fn empty_tool_result_shows_a_placeholder() {
        let p = Palette::default();
        let e = Entry::ToolResult {
            call_id: "c".into(),
            name: "bash".into(),
            ok: true,
            result: "".into(),
        };
        let all = text_of(&render(&[e], &p, false, false));
        assert!(all.contains("(no output)"), "空结果必须有占位符: {all}");
    }

    #[test]
    fn tool_result_payload_stores_data_not_view() {
        // Persistence contract: the tool_result payload stores data only (call_id/name/ok/result),
        // the view is a derived UI concept synthesized at render time by ToolView::synthesize
        let e = Entry::ToolResult {
            call_id: "c9".into(),
            name: "edit".into(),
            ok: true,
            result: "- a\n+ b".into(),
        };
        let (_, payload) = e.to_payload();
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(v.get("view").is_none(), "payload 不该有 view 字段");
        assert_eq!(v.get("result").and_then(|r| r.as_str()), Some("- a\n+ b"));
        // After the round-trip the Diff is re-synthesized from data
        let back = Entry::from_payload("tool_result", &payload).unwrap();
        match ToolView::synthesize("edit", true, "- a\n+ b") {
            ToolView::Diff {
                deletions,
                insertions,
            } => {
                assert_eq!(deletions, vec!["a"]);
                assert_eq!(insertions, vec!["b"]);
            }
            other => panic!("应合成出 Diff: {other:?}"),
        }
        assert_eq!(back, e);
    }
}
