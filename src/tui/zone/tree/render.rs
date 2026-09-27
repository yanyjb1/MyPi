//! 节点树的渲染：几何 → 字符。
//!
//! **一块一行**（紧凑布局）：`user: 前几个字…` 这样。不换行、不多占行。
//! 分支关系全靠**行首前缀**——祖先岔路口的竖线 + 本层连接符，规则在
//! [`super::layout`]，这里只负责把它画出来。
//!
//! 宽度按**显示列**算（`unicode_width`），不是字符数：中文一个字占两列，
//! 按 `chars().count()` 算会让整行溢出、把竖线挤歪。
//!
//! 颜色：**五类角色各占一个色**（用户定——树的意义就是一眼看出这段对话
//! 是谁说的，同色等于没标）。色号全部走 [`HistoryTheme`]，换主题跟着变。

use super::Row;
use super::layout::{COL_STEP, RowGeom};
use crate::tui::zone::TermSize;
use crate::tui::zone::main::history::render::theme::{HistoryTheme, Token};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

/// 竖线：祖先岔路口垂下来的那一支。
const BAR: char = '│';
/// 连接符左端：后面还有兄弟。
const TEE: char = '├';
/// 连接符左端：最后一个兄弟。
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

/// 前缀：祖先岔路口的竖线 + 本层连接符 + 缩进留白。
///
/// 结构线一色、留白无色，所以攒成两段交替推，不是每个字符一个 span
/// （树每帧都要全量重画，span 数是热路径上的常数）。
fn prefix(geom: &RowGeom, spans: &mut Vec<Span<'static>>, axis: Style) {
    let conn_level = geom.connector_level();
    let mut bars = String::new();
    let mut blanks = String::new();
    for i in 0..geom.prefix_cols() {
        let level = i / COL_STEP;
        let pos = i % COL_STEP;
        let ch = if let Some(show) = geom.gutter(level) {
            // 祖先的岔路口：竖线只占层首那一列。
            (pos == 0 && show).then_some(BAR)
        } else if conn_level == Some(level) {
            match pos {
                0 => Some(if geom.connector == Some(true) {
                    CORNER
                } else {
                    TEE
                }),
                1 => Some(DASH),
                _ => None,
            }
        } else {
            None
        };
        match ch {
            Some(c) => {
                if !blanks.is_empty() {
                    spans.push(Span::raw(std::mem::take(&mut blanks)));
                }
                bars.push(c);
            }
            None => {
                if !bars.is_empty() {
                    spans.push(Span::styled(std::mem::take(&mut bars), axis));
                }
                blanks.push(' ');
            }
        }
    }
    if !blanks.is_empty() {
        spans.push(Span::raw(std::mem::take(&mut blanks)));
    }
    if !bars.is_empty() {
        spans.push(Span::styled(std::mem::take(&mut bars), axis));
    }
}

/// 一块一行：前缀（树形）接本块文字，按列宽截断。
///
/// 前缀里的竖线和横线是**暗淡**的（它们是结构，不是内容）；角色名用该角色的
/// 色，摘要用正文色。这样一眼扫下去能看出"谁说的话"。
fn paint(row: &Row, cols: usize, t: &HistoryTheme) -> Line<'static> {
    let axis = t.fg_style(Token::TreeAxis);
    let mut spans: Vec<Span<'static>> = Vec::new();
    prefix(&row.geom, &mut spans, axis);

    // 正文：`role: 摘要`。角色名上该角色的色，摘要走正文色。
    let used: usize = spans.iter().map(|s| s.content.width()).sum();
    let body_cols = cols.saturating_sub(used).max(1);
    let (role, rest) = split_role(&row.text);
    // 角色名 + ": " 的宽度，摘要只剩这么多列。
    let role_cols = role.width() + 2;
    if role_cols + 1 < body_cols {
        spans.push(Span::styled(role.to_string(), role_style(&row.kind, t)));
        spans.push(Span::raw(": "));
        let rest_cols = body_cols - role_cols;
        spans.push(Span::styled(
            fit(rest, rest_cols),
            t.fg_style(Token::AssistantText),
        ));
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

/// 角色 → 色号。五类互不相同（用户定的）。
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::zone::tree::Row;
    use crate::tui::zone::tree::layout::{ForkRef, NodeRef};
    use crate::tui::zone::{TreeZone, ZoneId};

    fn theme() -> HistoryTheme {
        HistoryTheme::resolve()
    }

    /// 场景：主干 A B  ->  从块 2 岔出 f2  ->  f2 两块（块 3,4）
    ///        ->  从块 4 岔出 f3  ->  f3 一块（块 5）  ->  回主干 C D。
    fn sample() -> Vec<Row> {
        let forks = vec![
            ForkRef {
                id: 1,
                parent: None,
                fork_block: None,
            },
            ForkRef {
                id: 2,
                parent: Some(1),
                fork_block: Some(2),
            },
            ForkRef {
                id: 3,
                parent: Some(2),
                fork_block: Some(4),
            },
        ];
        let spec: Vec<(i64, i64, &str, &str)> = vec![
            (1, 1, "user", "user: 主干 A"),
            (2, 1, "user", "user: 主干 B"),
            (3, 2, "assistant", "assistant: 分1 a"),
            (4, 2, "tool_result", "tool_result: 分1 b"),
            (5, 3, "reasoning", "reasoning: 分2 a"),
            (6, 1, "user", "user: 主干 C"),
            (7, 1, "assistant", "assistant: 主干 D"),
        ];
        let nodes: Vec<NodeRef> = spec
            .iter()
            .map(|(id, fk, _, _)| NodeRef {
                block: *id,
                session: *fk,
            })
            .collect();
        crate::tui::zone::tree::layout::rows(&forks, &nodes)
            .into_iter()
            .map(|p| {
                let (id, fk, kind, tx) = &spec[p.src];
                Row {
                    geom: p.geom,
                    block_id: *id,
                    text: tx.to_string(),
                    fork: *fk,
                    kind: kind.to_string(),
                }
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
        assert_eq!(painted(&sample()).len(), 7);
    }

    /// 主干**齐平在左**：一条直线上的块不带任何前缀。
    #[test]
    fn the_trunk_stays_flush_left() {
        let lines = painted(&sample());
        assert_eq!(lines[0], "user: 主干 A");
        assert_eq!(lines[1], "user: 主干 B");
    }

    /// 分支那一行带连接符，分支内部带祖先的竖线（一直垂到那一支结束）。
    #[test]
    fn a_branch_row_draws_its_connector_and_gutter() {
        let lines = painted(&sample());
        assert!(lines[2].starts_with("├─ "), "分支头：{}", lines[2]);
        assert!(lines[3].starts_with("│  "), "分支内部要竖线：{}", lines[3]);
        assert!(lines[4].starts_with("│  "), "再深一层也还得有：{}", lines[4]);
        assert!(lines[5].starts_with("└─ "), "最后一条分支：{}", lines[5]);
    }

    /// 末支下面不再竖线：`└─` 之后是空格，不是 `│`。
    #[test]
    fn the_last_branch_has_no_gutter() {
        let lines = painted(&sample());
        assert!(!lines[5].contains(BAR), "末支自己不带竖线：{}", lines[5]);
        assert!(
            !lines[6].contains(BAR),
            "末支内部也不该有竖线：{}",
            lines[6]
        );
        assert_eq!(lines[6], "   assistant: 主干 D", "留白对齐，不画线");
    }

    /// **每一类角色的颜色都必须互不相同。**
    ///
    /// 以前整棵树一个色，分不清谁说的话。`tool_call` 和 `tool_result` 也在内
    /// ——它们是这条往返的两半，同色等于又把它们合并回去了。
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

    /// 结构线（竖线/横线）和内容色**不同**：结构不该抢正文的风头。
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
            geom: RowGeom {
                indent: 1,
                connector: Some(false),
                gutters: Vec::new(),
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
        assert_eq!(z.selected, 6);
        z.deliver(k(KeyCode::Down));
        assert_eq!(z.selected, 6, "底端再往下不动");
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
        for (i, l) in painted(&sample()).iter().enumerate() {
            println!("{i}: {l}");
        }
    }
}
