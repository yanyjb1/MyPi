//! 节点树的渲染：几何 → 字符。
//!
//! **一块一行**（紧凑布局，用户定的）：`user: 前几个字…` 这样。不换行、不
//! 多占行 —— 分叉也是行内的两条竖线加一个 `├`，不另起一行。
//!
//! 宽度按**显示列**算（`unicode_width`），不是字符数：中文一个字占两列，
//! 按 `chars().count()` 算会让整行溢出、把竖线挤歪。
//!
//! 颜色：**四类角色各占一个色**（用户定的——树的意义就是一眼看出这段对话
//! 是谁说的，同色等于没标）。色号全部走 [`HistoryTheme`]，换主题跟着变。

use super::Row;
use crate::tui::zone::TermSize;
use crate::tui::zone::main::history::render::theme::{HistoryTheme, Token};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

/// 竖直轴。
const BAR: char = '│';
/// 分叉拐点：竖线在中、往右岔——它**本身含主干那根竖线**，所以主干不断。
const TEE: char = '├';
/// 子轴顶端：横在左、竖在下，收住子轴。
const TOP: char = '┐';
/// 在**分支**上分叉的拐点：竖在上、横在右，下边不出头。
const CORNER: char = '└';
/// 横线。
const DASH: char = '─';
/// 截断标记。
const ELLIPSIS: char = '…';

/// 画可见的那一页。一块一行。
pub fn rows(
    rows: &[Row],
    selected: usize,
    size: Option<TermSize>,
    scroll: &mut usize,
    t: &HistoryTheme,
) -> Vec<Line<'static>> {
    let height = size.map(|s| s.rows as usize).unwrap_or(24);
    let cols = size.map(|s| s.cols as usize).unwrap_or(80);
    // 顶部留一行提示。
    let body = height.saturating_sub(1);

    if rows.is_empty() {
        return vec![Line::from(" （空）")];
    }

    // 一块一行：滚动窗口以行为单位。
    if selected < *scroll {
        *scroll = selected;
    } else if selected >= *scroll + body {
        *scroll = selected + 1 - body;
    }
    *scroll = (*scroll).min(rows.len().saturating_sub(body));

    let mut out = Vec::with_capacity(body + 1);
    out.push(Line::from(" ↑↓ 选块   Enter 回到这块   Esc 关 "));
    for (i, row) in rows.iter().enumerate().skip(*scroll).take(body) {
        out.push(highlight(paint(row, cols, t), i == selected, cols));
    }
    out
}

/// 高亮那一行整条反白（颜色保留）。
fn highlight(line: Line<'static>, on: bool, cols: usize) -> Line<'static> {
    if !on {
        return line;
    }
    let used = line.width();
    let mut spans = line.spans;
    if used < cols {
        spans.push(Span::raw(" ".repeat(cols - used)));
    }
    Line::from(spans).style(Style::new().add_modifier(Modifier::REVERSED))
}

/// 一块一行：前缀（轴 / 分叉）接本块文字，按列宽截断。
///
/// 前缀里的轴和连线是**暗淡**的（它们是结构，不是内容）；角色名用该角色的
/// 色，摘要用正文色。这样一眼扫下去能看出"谁说的话"。
fn paint(row: &Row, cols: usize, t: &HistoryTheme) -> Line<'static> {
    // 结构线：轴亮一档，横线更暗。
    let axis = t.fg_style(Token::TreeAxis);
    let line_st = t.fg_style(Token::TreeLine);

    let col = indent_cols(row.geom.column);
    let child = indent_cols(row.geom.child_column);
    let mut spans: Vec<Span<'static>> = Vec::new();

    // 主干恒在 0 列：分叉时它由 `├` 承担（`├` 含竖线）。
    if col == 0 && row.geom.forks_here {
        spans.push(Span::styled(TEE.to_string(), axis));
        spans.push(Span::styled(
            DASH.to_string().repeat(child.saturating_sub(1)),
            line_st,
        ));
        spans.push(Span::styled(TOP.to_string(), axis));
    } else {
        spans.push(Span::styled(BAR.to_string(), axis));
        if col > 1 {
            spans.push(Span::raw(" ".repeat(col - 1)));
        }
        if row.geom.forks_here {
            // 分支上分叉：本块的轴已经在上面了，拐点紧接其后，用不出头的 `└`。
            spans.push(Span::styled(CORNER.to_string(), axis));
            spans.push(Span::styled(
                DASH.to_string().repeat(child.saturating_sub(col + 1)),
                line_st,
            ));
            spans.push(Span::styled(TOP.to_string(), axis));
        } else if col > 0 {
            spans.push(Span::styled(BAR.to_string(), axis));
        }
    }
    spans.push(Span::raw(" "));

    // 正文：`role: 摘要`。角色名上该角色的色，摘要走正文色。
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    let body_cols = cols.saturating_sub(used + 1).max(1);
    let (role, rest) = split_role(&row.text);
    // 角色名 + ": " 的宽度，摘要只剩这么多列。
    let role_cols = role.width() + 2;
    if role_cols + 1 < body_cols {
        spans.push(Span::styled(role.to_string(), role_style(&row.kind, t)));
        spans.push(Span::raw(": "));
        let rest_cols = body_cols - role_cols;
        spans.push(Span::styled(fit(rest, rest_cols), t.fg_style(Token::AssistantText)));
    } else {
        // 太窄，连角色名都放不下：整段按角色色截断。
        spans.push(Span::styled(
            fit(&row.text, body_cols),
            role_style(&row.kind, t),
        ));
    }
    Line::from(spans)
}

/// `"user: 正文"` → `("user", "正文")`。没有冒号时整个当正文、角色空。
fn split_role(text: &str) -> (&str, &str) {
    match text.split_once(": ") {
        Some((r, rest)) => (r, rest),
        None => ("", text),
    }
}

/// 角色 → 色号。四类互不相同（用户定的）。
fn role_style(kind: &str, t: &HistoryTheme) -> Style {
    let tok = match kind {
        "user" => Token::TreeUser,
        "assistant" => Token::TreeAssistant,
        "tool_call" => Token::TreeToolCall,
        "tool_result" => Token::TreeToolResult,
        "reasoning" => Token::TreeReasoning,
        _ => Token::Dim,
    };
    t.fg_style(tok).add_modifier(Modifier::BOLD)
}

/// 按**显示列**截断到 `max` 列，超了补 `…`。
fn fit(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    let (cut, _) = crate::tui::text::take_width(s, max.saturating_sub(1));
    let mut out = cut;
    out.push(ELLIPSIS);
    out
}

/// 第 `column` 列对应的字符列。列宽的唯一定义在 `layout::COL_STEP`。
fn indent_cols(column: usize) -> usize {
    column * crate::tui::zone::tree::layout::COL_STEP
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::zone::tree::Row;
    use crate::tui::zone::tree::layout::{ForkRef, NodeRef};
    use crate::tui::zone::{TreeZone, ZoneId};

    fn theme() -> HistoryTheme {
        HistoryTheme::resolve()
    }

    /// 场景：主干(1) A B  ->  从 B（块 2）岔出 f2  ->  f2 两块（块 3,4）
    ///        ->  从块 4 岔出 f3  ->  f3 两块（块 5,6）  ->  回主干 C D。
    fn sample() -> Vec<Row> {
        let forks = vec![
            ForkRef { id: 1, parent: None, fork_block: None },
            ForkRef { id: 2, parent: Some(1), fork_block: Some(2) },
            ForkRef { id: 3, parent: Some(2), fork_block: Some(4) },
        ];
        let spec: Vec<(i64, i64, &str, &str)> = vec![
            (1, 1, "user", "user: 主干 A"),
            (2, 1, "user", "user: 主干 B"),
            (3, 2, "assistant", "assistant: 分1 a"),
            (4, 2, "tool_result", "tool_result: 分1 b"),
            (5, 3, "reasoning", "reasoning: 分2 a"),
            (6, 3, "user", "user: 分2 b"),
            (7, 1, "user", "user: 主干 C"),
            (8, 1, "assistant", "assistant: 主干 D"),
        ];
        let nodes: Vec<NodeRef> = spec
            .iter()
            .enumerate()
            .map(|(i, (_, fk, _, _))| NodeRef { index: i, fork: *fk })
            .collect();
        let ids: Vec<i64> = spec.iter().map(|(id, _, _, _)| *id).collect();
        let geo = crate::tui::zone::tree::layout::rows(&forks, &nodes, |i| ids[i]);
        geo.into_iter()
            .zip(spec)
            .map(|(geom, (id, fk, kind, tx))| Row {
                geom,
                block_id: id,
                text: tx.to_string(),
                fork: fk,
                kind: kind.to_string(),
            })
            .collect()
    }

    /// 每一块画成它该有的那一行（一块一行）。
    fn painted(rows: &[Row]) -> Vec<String> {
        let t = theme();
        rows.iter().map(|r| paint(r, 80, &t).to_string()).collect()
    }

    #[test]
    fn every_block_is_exactly_one_line() {
        assert_eq!(painted(&sample()).len(), 8);
    }

    #[test]
    fn the_trunk_bar_is_never_broken() {
        for (i, l) in painted(&sample()).iter().enumerate() {
            assert!(
                l.starts_with(BAR) || l.starts_with(TEE),
                "第 {i} 行丢了主干：{l:?}"
            );
        }
    }

    #[test]
    fn a_row_never_carries_more_than_two_bars() {
        for l in painted(&sample()) {
            let n = l.matches(BAR).count();
            assert!(n <= 2, "一行超过两根竖线：{l:?} ({n})");
        }
    }

    #[test]
    fn a_fork_row_connects_the_corner_to_the_child_axis() {
        let lines = painted(&sample());
        let f = lines.iter().find(|l| l.contains(TEE)).expect("该有分叉");
        let cs: Vec<char> = f.chars().collect();
        let tee = cs.iter().position(|c| *c == TEE).unwrap();
        let top = cs.iter().position(|c| *c == TOP).unwrap();
        let between = &cs[tee + 1..top];
        assert!(
            !between.is_empty() && between.iter().all(|c| *c == DASH),
            "拐点到子轴之间断了：{f:?}"
        );
    }

    /// **每一类角色的颜色都必须互不相同。**
    ///
    /// 这条是这次改动的主要目的：以前整棵树一个色，分不清谁说的话。
    /// `tool_call` 和 `tool_result` 也在内——它们是这条往返的两半，
    /// 同色等于又把它们合并回去了。
    #[test]
    fn every_role_gets_its_own_color() {
        let t = theme();
        let fg = |kind: &str| role_style(kind, &t).fg;
        let mut seen = std::collections::HashMap::new();
        for k in ["user", "assistant", "tool_call", "tool_result", "reasoning"] {
            let c = fg(k).unwrap_or_else(|| panic!("{k} 没有前景色"));
            assert!(
                seen.insert(c, k).is_none(),
                "{k} 的颜色 {c:?} 和 {:?} 撞了",
                seen.get(&c)
            );
        }
    }

    /// 角色名上的色确实落到了 span 上（不是只算不用）。
    #[test]
    fn the_role_name_span_carries_its_own_color() {
        let t = theme();
        let rows = sample();
        let user_row = rows.iter().find(|r| r.kind == "user").unwrap();
        let line = paint(user_row, 80, &t);
        let want = t.fg_style(Token::TreeUser).fg;
        assert!(
            line.spans.iter().any(|s| s.style.fg == want),
            "user 行里没有 user 色的 span：{:?}",
            line.spans
        );
    }

    /// 结构线（轴）和内容色**不同**：轴不该抢正文的风头。
    #[test]
    fn the_structure_lines_use_their_own_color() {
        let t = theme();
        let axis = t.fg_style(Token::TreeAxis).fg;
        let text = t.fg_style(Token::AssistantText).fg;
        assert_ne!(axis, text, "轴色和正文色撞了");
    }

    /// **中文按显示列算宽度**：一行绝不超过终端列数。
    #[test]
    fn cjk_text_never_overflows_the_terminal_width() {
        let t = theme();
        let rows = [Row {
            geom: crate::tui::zone::tree::layout::RowGeom {
                column: 0,
                forks_here: false,
                child_column: 0,
            },
            block_id: 1,
            text: format!("user: {}", "中文内容很长很长".repeat(20)),
            fork: 1,
            kind: "user".into(),
        }];
        for cols in [20usize, 33, 40, 80] {
            let l = paint(&rows[0], cols, &t);
            assert!(
                l.width() <= cols,
                "宽度 {cols} 溢出了（实际 {}）：{l:?}",
                l.width()
            );
        }
    }

    #[test]
    fn a_long_summary_gets_an_ellipsis_and_a_short_one_does_not() {
        let long = fit("一二三四五六七八九十", 6);
        assert!(long.ends_with(ELLIPSIS), "截断了要有省略号：{long:?}");
        assert!(long.width() <= 6);
        assert_eq!(fit("短", 10), "短", "没超就不要动");
    }

    #[test]
    fn selection_clamps_at_both_ends() {
        use crate::tui::zone::Zone as _;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let k = |c| crate::tui::keys::RawEvent::Key {
            key: KeyEvent::new(c, KeyModifiers::NONE),
        };
        let mut z = TreeZone::default();
        z.attach(crate::tui::zone::TermSize { cols: 40, rows: 10 });
        z.begin(sample(), 0);
        z.deliver(k(KeyCode::Up));
        assert_eq!(z.selected, 0, "顶端再往上不动");
        z.deliver(k(KeyCode::End));
        assert_eq!(z.selected, 7);
        z.deliver(k(KeyCode::Down));
        assert_eq!(z.selected, 7, "底端再往下不动");
    }

    #[test]
    fn esc_and_enter_leave_the_zone() {
        use crate::tui::zone::Zone as _;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        for code in [KeyCode::Esc, KeyCode::Enter] {
            let mut z = TreeZone::default();
            z.attach(crate::tui::zone::TermSize { cols: 40, rows: 10 });
            z.begin(sample(), 3);
            let h = z.deliver(crate::tui::keys::RawEvent::Key {
                key: KeyEvent::new(code, KeyModifiers::NONE),
            });
            assert_eq!(
                h.map(|h| h.new_owner),
                Some(ZoneId::Main),
                "{code:?} 必须把所有权交回主区"
            );
        }
    }

    #[test]
    fn enter_reports_the_block_to_return_to() {
        use crate::tui::zone::Zone as _;
        use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut z = TreeZone::default();
        z.attach(crate::tui::zone::TermSize { cols: 40, rows: 10 });
        z.begin(sample(), 3);
        z.deliver(crate::tui::keys::RawEvent::Key {
            key: KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        });
        assert_eq!(z.take_return_to(), Some(4), "选中的是块 4");
        assert_eq!(z.take_return_to(), None, "取走一次就没了");
    }

    #[test]
    fn print_the_tree() {
        let t = theme();
        for r in &sample() {
            println!("{}", paint(r, 80, &t));
        }
    }
}
