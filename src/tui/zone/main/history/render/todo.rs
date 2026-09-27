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
use crate::server::ai::config::TodoFold;
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

/// 画整块（不含占用几行的裁剪；`budget` 是它能用的最大行数）。
///
/// `budget` 太小就砍尾巴并补一行 `… 还有 N 行`——宁可少画也不能把
/// 聊天区顶没。
///
/// `fold` 是折叠策略（`config.yaml` → `tui.TodoFold`），`touched` 是**最近
/// 一次 `todo` 调用碰过的阶段名**。折叠开着时，没被碰过的阶段折成一行
/// `名字 · 3/5`，碰过的完整展开——所以你在做大任务一，二和三收起来。
/// `touched` 为空 = 没有信号，全部展开（和关掉折叠一个效果）。
pub fn rows(
    phases: &[TodoPhase],
    touched: &[String],
    fold: TodoFold,
    width: usize,
    t: &HistoryTheme,
    budget: usize,
) -> Vec<Line<'static>> {
    if phases.is_empty() || budget == 0 {
        return Vec::new();
    }
    // 全收尾了就不占地方：这块是「正在做什么」的看板，活干完了它就该让位
    // 给对话。注意是**收尾**（完成或放弃）而不是「完成」——放弃的也不会再动。
    if phases
        .iter()
        .flat_map(|p| p.tasks.iter())
        .all(|x| x.status.is_closed())
    {
        return Vec::new();
    }
    // 折叠只在「多于一阶段」时才有意义：只有一个阶段时折起来等于什么都不显示。
    let folding = fold.is_on() && phases.len() > 1 && !touched.is_empty();
    let cap = fold.cap();
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
        let done = phase.tasks.iter().filter(|x| x.status.is_closed()).count();
        let progress = format!(" · {done}/{}", phase.tasks.len());
        // 折起来的两条理由：
        //   1. 这个阶段全收尾了——没剩活的，展开只是占地方；
        //   2. 折叠开着，而这次更新没碰过它——注意力在别处。
        let live: Vec<_> = phase.tasks.iter().filter(|x| !x.status.is_closed()).collect();
        let untouched = folding && !touched.iter().any(|n| n == &phase.name);
        let mut spans = vec![t.fg(format!(" {branch} "), Token::Dim)];
        if live.is_empty() || untouched {
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
        // `cap` 只裁**展开**的列表（折叠本来就是上限的目的），折起来的那行
        // 是摘要，不受影响。
        let shown = phase.tasks.len().min(cap);
        let cont = if last_phase { "" } else { TREE_VERTICAL };
        for ti in 0..shown {
            let task = &phase.tasks[ti];
            let sub = if ti + 1 == shown { TREE_LAST } else { TREE_BRANCH };
            let mut line = vec![t.fg(format!(" {cont}  {sub} "), Token::Dim)];
            line.extend(task_spans(t, task));
            body.push(Line::from(line));
        }
        if shown < phase.tasks.len() {
            let hidden = phase.tasks.len() - shown;
            body.push(Line::from(vec![
                t.fg(format!(" {cont}  "), Token::Dim),
                t.fg(format!("… 还有 {hidden} 条"), Token::Muted),
            ]));
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
        drawn(phases, &[], TodoFold::default(), budget)
    }

    /// 画一遍再看文本。默认 `touched` 空、折叠关——就是"全展开"。
    fn drawn(
        phases: &[TodoPhase],
        touched: &[String],
        fold: TodoFold,
        budget: usize,
    ) -> Vec<String> {
        rows(phases, touched, fold, 80, &HistoryTheme::resolve(), budget)
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

    /// 预算就是硬上限：画出来的行数**绝不**超过它。这块的行高是预留出去
    /// 的（聊天窗口因此变矮），多画一行就会顶掉一行聊天。
    #[test]
    fn the_drawn_height_never_exceeds_the_budget() {
        for budget in [0usize, 1, 2, 3, 4, 6, 40, 100] {
            let got = drawn(&sample(), &[], TodoFold::default(), budget);
            assert!(got.len() <= budget, "budget {budget} 画了 {} 行", got.len());
        }
        // 空清单不占地方。
        assert!(rows(&[], &[], TodoFold::default(), 80, &HistoryTheme::resolve(), 40).is_empty());
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
        let lines = rows(&phases, &[], TodoFold::default(), 80, &HistoryTheme::resolve(), 40);
        // 放弃的那条必须是删除线（红 + crossed out），不然和「待办」分不开。
        let aband = lines[4].spans.iter().any(|s| {
            s.style.add_modifier.contains(Modifier::CROSSED_OUT)
                && s.content.contains("不要了")
        });
        assert!(aband, "放弃的条目该有删除线");
    }

    /// 折叠开着时，**没碰过**的阶段折成一行、碰过的那条完整展开。
    /// 这就是「我在做第一个大任务，另外两个收起来」。
    #[test]
    fn fold_collapses_the_phases_this_update_did_not_touch() {
        let phases = three_phases();
        let touched = vec!["甲".to_string()];
        let got = drawn(&phases, &touched, TodoFold::On(true), 60);
        // 甲 展开，乙/丙 折成一行。
        assert!(got.iter().any(|l| l.contains("甲")), "{got:#?}");
        assert!(got.iter().any(|l| l.contains("[>] 做甲的活")), "{got:#?}");
        assert!(
            !got.iter().any(|l| l.contains("做乙的活")),
            "乙没被碰过，该折起来：{got:#?}"
        );
        assert!(
            !got.iter().any(|l| l.contains("做丙的活")),
            "丙没被碰过，该折起来：{got:#?}"
        );
        // 折起来的那两行仍要报进度。
        assert!(got.iter().any(|l| l.contains("乙") && l.contains("0/1")), "{got:#?}");
        assert!(got.iter().any(|l| l.contains("丙") && l.contains("0/1")), "{got:#?}");
    }

    /// 折叠关着（默认）时全部展开——和早先的行为一致。
    #[test]
    fn fold_off_expands_every_phase() {
        let phases = three_phases();
        let touched = vec!["甲".to_string()];
        let got = drawn(&phases, &touched, TodoFold::On(false), 60);
        for needle in ["做甲的活", "做乙的活", "做丙的活"] {
            assert!(got.iter().any(|l| l.contains(needle)), "缺 {needle}：{got:#?}");
        }
    }

    /// 折叠开着但**没有任何触碰信号**（老转录）= 全展开，不能误折。
    #[test]
    fn fold_with_no_signal_expands_everything() {
        let got = drawn(&three_phases(), &[], TodoFold::On(true), 60);
        for needle in ["做甲的活", "做乙的活", "做丙的活"] {
            assert!(got.iter().any(|l| l.contains(needle)), "缺 {needle}：{got:#?}");
        }
    }

    /// 数字上限只裁**展开**的那个阶段，多出来的报条数。
    #[test]
    fn fold_cap_trims_the_expanded_phase_only() {
        let phases = vec![
            TodoPhase {
                name: "甲".into(),
                tasks: (1..=5).map(|i| task(&format!("活{i}"), TodoStatus::Pending)).collect(),
            },
            TodoPhase {
                name: "乙".into(),
                tasks: vec![task("乙的活", TodoStatus::Pending)],
            },
        ];
        let touched = vec!["甲".to_string()];
        let got = drawn(&phases, &touched, TodoFold::Capped(2), 60);
        assert!(got.iter().any(|l| l.contains("活1")), "{got:#?}");
        assert!(got.iter().any(|l| l.contains("活2")), "{got:#?}");
        assert!(!got.iter().any(|l| l.contains("活3")), "超上限该裁掉：{got:#?}");
        assert!(
            got.iter().any(|l| l.contains("还有 3 条")),
            "裁掉几条要如实报：{got:#?}"
        );
        // 乙 没被碰过，折成一行——注意它不在展开上限的账上。
        assert!(!got.iter().any(|l| l.contains("乙的活")), "{got:#?}");
    }

    /// 清单一做完（全部收尾）就**整块消失**，一行都不占。
    ///
    /// 这块是"正在做什么"的看板，活干完了就该让位给对话。放弃也算收尾——
    /// 它不会再动了。
    #[test]
    fn the_list_disappears_once_everything_is_closed() {
        let mut phases = sample();
        for p in &mut phases {
            for t in &mut p.tasks {
                t.status = TodoStatus::Done;
            }
        }
        assert!(
            rows(&phases, &[], TodoFold::default(), 80, &HistoryTheme::resolve(), 40).is_empty(),
            "全做完了还占着地方"
        );
        // 放弃也算收尾。
        let mut dropped = sample();
        for p in &mut dropped {
            for t in &mut p.tasks {
                t.status = TodoStatus::Abandoned;
            }
        }
        assert!(
            rows(&dropped, &[], TodoFold::default(), 80, &HistoryTheme::resolve(), 40).is_empty(),
            "全放弃也算收尾"
        );
        // 只要还剩一条没做完，整块就还在。
        let mut one_left = sample();
        for p in &mut one_left {
            for t in &mut p.tasks {
                t.status = TodoStatus::Done;
            }
        }
        one_left[0].tasks[0].status = TodoStatus::Pending;
        assert!(
            !rows(&one_left, &[], TodoFold::default(), 80, &HistoryTheme::resolve(), 40).is_empty(),
            "还剩一条就得继续显示"
        );
    }

    fn three_phases() -> Vec<TodoPhase> {
        vec![
            TodoPhase {
                name: "甲".into(),
                tasks: vec![task("做甲的活", TodoStatus::InProgress)],
            },
            TodoPhase {
                name: "乙".into(),
                tasks: vec![task("做乙的活", TodoStatus::Pending)],
            },
            TodoPhase {
                name: "丙".into(),
                tasks: vec![task("做丙的活", TodoStatus::Pending)],
            },
        ]
    }
}
