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

use super::layout::{COL_STEP, RowGeom};
use super::rows::Row;
use crate::ui::tui::theme::{HistoryTheme, Token};
use crate::ui::tui::zone::TermSize;
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
    let (cut, _) = crate::ui::tui::text::take_width(s, max.saturating_sub(1));
    let mut out = cut;
    out.push(ELLIPSIS);
    out
}
