//! 任务清单的树形画法（置底那块）。
//!
//! 形状照 omp 的 `packages/tui/src/tools/todo.ts` + `render/tree-list.ts`：
//!
//! ```text
//! TODO
//!  |-- IV. 事件分层 · 3/4
//!  |  |-- [x] 耗时：start/end 带时间戳
//!  |  '-- [>] SessionEvent 切成两半
//!  |-- V. 配置与 profile · 3/3
//!  `----
//! ```
//!
//! 三条规矩都来自 omp：
//! - 阶段用**罗马数字**编号（`IV.`），那是显示层的事，状态和给模型的文本
//!   里从来没有它；
//! - 已经收尾的阶段折成一行 `名字 · 3/3`（全做完的显然不用再占地方），
//!   还有活的阶段展开；
//! - 已完成的条目加删除线，进行中是 accent，放弃是红删除线，阻塞带原因。
//!
//! 这块是**贴底**的：它在历史区自己的行高里占最后几行，聊天窗口相应变矮。

use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

use super::cards::clip_spans;
use super::glyphs::{
    TODO_ACTIVE, TODO_BLOCKED, TODO_DONE, TODO_DROPPED, TODO_PENDING, TREE_BRANCH, TREE_HOOK,
    TREE_LAST, TREE_VERTICAL,
};
use super::theme::{HistoryTheme, Token};
use crate::server::entry::{TodoPhase, TodoStatus};

/// 罗马数字（1 起）。显示层专用：状态与提示词里从来看不到它。
///
/// omp 同款的贪心表——数字大到用不上，但写全了就不用猜边界。
pub fn roman(one_based: usize) -> String {
    const PAIRS: [(usize, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut n = one_based;
    let mut out = String::new();
    for (v, sym) in PAIRS {
        while n >= v {
            out.push_str(sym);
            n -= v;
        }
    }
    out
}

/// 一条任务算不算「收尾」（完成或放弃）。
///
/// 折起来的阶段按这个数进度：放弃的任务也算收尾——它不会再动了，
/// 不计的话那个阶段的 `N/M` 会永远停在没做完的样子。
pub fn is_closed(status: TodoStatus) -> bool {
    matches!(status, TodoStatus::Done | TodoStatus::Abandoned)
}

/// 画整块（不含占用几行的裁剪；`budget` 是它能用的最大行数）。
///
/// `budget` 太小就砍尾巴并补一行 `… 还有 N 行`——宁可少画也不能把
/// 聊天区顶没。
pub fn rows(
    phases: &[TodoPhase],
    width: usize,
    t: &HistoryTheme,
    budget: usize,
) -> Vec<Line<'static>> {
    if phases.is_empty() || budget == 0 {
        return Vec::new();
    }
    let clip = |line: Line<'static>| -> Line<'static> {
        if line.width() <= width {
            return line;
        }
        let style = line.style;
        Line::from(clip_spans(line.spans, width)).style(style)
    };
    let title = t.fg_mod("TODO", Token::ToolTitle, Modifier::BOLD);
    // 标题 + 树 + 收尾那行 `---- 至少三行才有画的意义。
    if budget < 3 {
        return vec![clip(Line::from(title))];
    }
    let mut body: Vec<Line<'static>> = Vec::new();
    let total = phases.len();
    for (idx, phase) in phases.iter().enumerate() {
        let last_phase = idx + 1 == total;
        let branch = if last_phase { TREE_LAST } else { TREE_BRANCH };
        let name = format!("{}. {}", roman(idx + 1), phase.name);
        let done = phase.tasks.iter().filter(|x| is_closed(x.status)).count();
        let progress = format!(" · {done}/{}", phase.tasks.len());
        // 全收尾的阶段折成一行：没剩活的，展开只是占地方。
        let live: Vec<_> = phase.tasks.iter().filter(|x| !is_closed(x.status)).collect();
        let mut spans = vec![t.fg(format!(" {branch} "), Token::Dim)];
        if live.is_empty() {
            // 折起来：整行 dim（omp 的 `formatPhaseSummary`）。
            spans.push(t.fg_mod(name, Token::Dim, Modifier::BOLD));
            spans.push(t.fg(progress, Token::Dim));
            body.push(Line::from(spans));
            continue;
        }
        spans.push(t.fg_mod(name, Token::ToolTitle, Modifier::BOLD));
        spans.push(t.fg(progress, Token::Dim));
        body.push(Line::from(spans));
        // 展开：每条任务一行，挂在这个阶段的竖线下面。
        let cont = if last_phase { "" } else { TREE_VERTICAL };
        let n = phase.tasks.len();
        for (ti, task) in phase.tasks.iter().enumerate() {
            let sub = if ti + 1 == n { TREE_LAST } else { TREE_BRANCH };
            let mut line = vec![t.fg(format!(" {cont}  {sub} "), Token::Dim)];
            line.extend(task_spans(t, task));
            body.push(Line::from(line));
        }
    }
    body.push(Line::from(t.fg(
        format!(" {TREE_HOOK}{}", "-".repeat(3)),
        Token::Dim,
    )));

    // 装不下：砍尾巴 + 一行说明。砍掉的行数如实报出来。
    let mut out = vec![Line::from(title)];
    let room = budget - 1;
    if body.len() <= room {
        out.extend(body);
        return out.into_iter().map(clip).collect();
    }
    let hidden = body.len() - (room - 1);
    out.extend(body.into_iter().take(room - 1));
    out.push(Line::from(t.fg(format!(" … 还有 {hidden} 行"), Token::Muted)));
    out.into_iter().map(clip).collect()
}

/// 一条任务：状态字形 + 文本（按状态上色/加删除线）。
fn task_spans(
    t: &HistoryTheme,
    task: &crate::server::entry::TodoTask,
) -> Vec<Span<'static>> {
    let (glyph, tok, strike) = match task.status {
        TodoStatus::Done => (TODO_DONE, Token::Dim, true),
        TodoStatus::InProgress => (TODO_ACTIVE, Token::ToolEdgePending, false),
        TodoStatus::Blocked => (TODO_BLOCKED, Token::Warn, false),
        TodoStatus::Abandoned => (TODO_DROPPED, Token::Err, true),
        TodoStatus::Pending => (TODO_PENDING, Token::AssistantText, false),
    };
    let mut style = ratatui::style::Style::new().fg(t.get(tok));
    if strike {
        style = style.add_modifier(Modifier::CROSSED_OUT);
    }
    let mut out = vec![Span::styled(format!("{glyph} "), style)];
    out.push(Span::styled(task.content.clone(), style));
    if let Some(why) = &task.blocker {
        out.push(t.fg(format!("（{why}）"), Token::Warn));
    }
    out
}

/// 这块要几行（渲染前先算高度用）。
pub fn height(phases: &[TodoPhase], budget: usize) -> usize {
    if phases.is_empty() || budget == 0 {
        return 0;
    }
    if budget < 3 {
        return 1;
    }
    let mut n = 1; // 标题
    let total = phases.len();
    for (idx, phase) in phases.iter().enumerate() {
        let _ = total;
        let _ = idx;
        n += 1; // 阶段行
        if phase.tasks.iter().any(|x| !is_closed(x.status)) {
            n += phase.tasks.len();
        }
    }
    n + 1 // `----
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::entry::{TodoPhase, TodoTask};

    fn task(content: &str, status: TodoStatus) -> TodoTask {
        TodoTask {
            content: content.into(),
            status,
            blocker: None,
        }
    }

    fn sample() -> Vec<TodoPhase> {
        vec![
            TodoPhase {
                name: "事件分层".into(),
                tasks: vec![
                    task("耗时：start/end 带时间戳", TodoStatus::Done),
                    task("SessionEvent 切成 Action / Event 两半", TodoStatus::Pending),
                ],
            },
            TodoPhase {
                name: "配置与 profile".into(),
                tasks: vec![
                    task("a", TodoStatus::Done),
                    task("b", TodoStatus::Done),
                    task("c", TodoStatus::Done),
                ],
            },
        ]
    }

    fn text(phases: &[TodoPhase], budget: usize) -> Vec<String> {
        rows(phases, 80, &HistoryTheme::resolve(), budget)
            .iter()
            .map(|l| l.to_string())
            .collect()
    }

    #[test]
    fn romans_count_like_a_person_expects() {
        assert_eq!(roman(1), "I");
        assert_eq!(roman(4), "IV");
        assert_eq!(roman(9), "IX");
        assert_eq!(roman(14), "XIV");
        assert_eq!(roman(0), "");
    }

    /// 抄的是 omp 的形状：阶段带罗马数字与进度，活着的阶段展开、收尾的折起来，
    /// 结尾一行收口。这条就是**看着屏幕**断言。
    #[test]
    fn the_tree_has_phases_tasks_and_a_closing_line() {
        let got = text(&sample(), 40);
        assert_eq!(
            got,
            vec![
                "TODO".to_string(),
                " |-- I. 事件分层 · 1/2".to_string(),
                " |  |-- [x] 耗时：start/end 带时间戳".to_string(),
                " |  '-- [ ] SessionEvent 切成 Action / Event 两半".to_string(),
                " '-- II. 配置与 profile · 3/3".to_string(),
                " `----".to_string(),
            ],
            "整块的形状"
        );
    }

    /// 高度和实际行数必须一致：这块的行高是**预留**出去的（聊天窗口因此
    /// 变矮），算多了会白留一片空白，算少了会把最后几行画到屏幕外。
    #[test]
    fn the_claimed_height_is_the_drawn_height() {
        for budget in [3usize, 4, 6, 40, 100] {
            let phases = sample();
            let drawn = rows(&phases, 80, &HistoryTheme::resolve(), budget).len();
            let claimed = height(&phases, budget);
            assert_eq!(drawn, claimed.min(budget), "budget {budget}");
        }
        // 空清单不占地方。
        assert_eq!(height(&[], 40), 0);
        assert!(rows(&[], 80, &HistoryTheme::resolve(), 40).is_empty());
    }

    /// 装不下时砍尾巴并**如实报**砍了几行，绝不超过预算。
    #[test]
    fn too_tall_cuts_the_tail_and_says_so() {
        let got = text(&sample(), 4);
        assert_eq!(got.len(), 4, "不超过预算");
        assert!(got[0].contains("TODO"));
        assert!(got[3].contains("还有"), "末行说明砍了几行：{}", got[3]);
    }

    #[test]
    fn status_glyphs_and_flavors_follow_the_state() {
        let phases = vec![TodoPhase {
            name: "阶段".into(),
            tasks: vec![
                task("正在做", TodoStatus::InProgress),
                task("卡住了", TodoStatus::Blocked),
                task("不要了", TodoStatus::Abandoned),
            ],
        }];
        let got = text(&phases, 40);
        assert!(got[2].contains("[>] 正在做"), "{}", got[2]);
        assert!(got[3].contains("[!] 卡住了"), "{}", got[3]);
        assert!(got[4].contains("[-] 不要了"), "{}", got[4]);
        let lines = rows(&phases, 80, &HistoryTheme::resolve(), 40);
        // 放弃的那条必须是删除线（红 + crossed out），不然和「待办」分不开。
        let aband = lines[4].spans.iter().any(|s| {
            s.style.add_modifier.contains(Modifier::CROSSED_OUT)
                && s.content.contains("不要了")
        });
        assert!(aband, "放弃的条目该有删除线");
    }
}
