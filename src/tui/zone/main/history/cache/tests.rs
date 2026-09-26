use super::*;
use crate::server::entry::Entry;

fn t() -> HistoryTheme {
    HistoryTheme::resolve()
}

/// `n` 块"用户 + 回答"，键从 1 开始（像库里的 block_id）。
fn convo(n: usize) -> Vec<(i64, Vec<Entry>)> {
    (0..n)
        .map(|i| {
            (
                i as i64 + 1,
                vec![
                    Entry::User {
                        content: format!("用户消息 {i} 一点内容"),
                    },
                    Entry::Assistant {
                        content: format!("回答 {i}"),
                        usage: None,
                    },
                ],
            )
        })
        .collect()
}

/// 一块 = 一条条目，键 `base + i`。
fn flat(entries: &[Entry], base: i64) -> Vec<(i64, Vec<Entry>)> {
    entries
        .iter()
        .enumerate()
        .map(|(i, e)| (base + i as i64, vec![e.clone()]))
        .collect()
}

fn items<'a>(bs: &'a [(i64, Vec<Entry>)]) -> Vec<Item<'a>> {
    bs.iter()
        .map(|(k, e)| Item {
            key: *k,
            entries: e,
        })
        .collect()
}

fn all(n: usize) -> Vec<usize> {
    (0..n).collect()
}

fn has(rows: &[ratatui::text::Line<'static>], needle: &str) -> bool {
    rows.iter()
        .any(|l| l.spans.iter().any(|s| s.content.contains(needle)))
}

fn text(rows: &[ratatui::text::Line<'static>]) -> String {
    rows.iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.trim_end().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn cold_start_renders_only_the_requested_window() {
    // The contract: a huge window + a bottom-window request must NOT render
    // the whole thing just to measure heights.
    let bs = convo(2000);
    let its = items(&bs);
    let mut c = BlockCache::new();
    c.sync(&its, 1, 60);
    let tail: Vec<usize> = (1990..2000).collect();
    let rows = c.rows_for(&its, &tail, &t(), true, false);
    assert!(!rows.is_empty());
    assert!(
        c.cached_blocks() <= 12,
        "冷启动必须只渲染拿出去的那几块：{}",
        c.cached_blocks()
    );
}

#[test]
fn a_sliding_window_never_crosses_rows_between_blocks() {
    // Positions shift on every page fetched — the window slides. Keys are
    // what keep the rows attached to the block they were rendered for.
    let bs = convo(30);
    let its = items(&bs);
    let mut c = BlockCache::new();
    c.sync(&its, 1, 60);
    let r0 = c.rows_for(&its, &(10..15).collect::<Vec<_>>(), &t(), true, false);
    assert!(has(&r0, "用户消息 10"));

    // The same keys, now at different positions (window slid forward).
    let slid: Vec<usize> = (5..20).collect();
    let r1 = c.rows_for(&its, &slid, &t(), true, false);
    assert!(has(&r1, "用户消息 10"), "滑窗之后同一块的行串了：{r1:?}");
    assert!(has(&r1, "用户消息 5") && has(&r1, "用户消息 19"));
    // The blocks that did not move were cache hits, not re-renders.
    assert!(c.diag.0 >= 5, "滑窗后该命中缓存：{:?}", c.diag);
}

#[test]
fn the_window_bounds_the_cache() {
    // Sync is the only place keys leave: the window is the bound, so a long
    // walk through ancient history cannot grow the cache.
    let bs = convo(400);
    let its = items(&bs);
    let mut c = BlockCache::new();
    c.set_budget(16);
    let mut end: usize = 400;
    while end > 0 {
        let start = end.saturating_sub(16);
        c.sync(&its[start..end], 1, 60);
        let _ = c.rows_for(&its, &all(end - start), &t(), true, false);
        end = start;
    }
    assert!(
        c.cached_blocks() <= 16,
        "LRU 必须按块数封顶：{}",
        c.cached_blocks()
    );
}

#[test]
fn heights_outlive_evicted_rows() {
    // The LRU drops rows; the heights stay, so scroll math over a window
    // whose rows were recycled is still exact (no re-render to re-measure).
    let bs = convo(60);
    let its = items(&bs);
    let mut c = BlockCache::new();
    c.set_budget(8);
    c.sync(&its, 1, 60);
    let h0 = c.height(&its[0], &t(), true, false);
    assert!(h0 > 0);
    // Render enough other blocks to push block 0's rows out of the cache.
    let _ = c.rows_for(&its, &(10..40).collect::<Vec<_>>(), &t(), true, false);
    assert!(c.cached_blocks() <= 8, "LRU 没按预算收：{}", c.cached_blocks());
    let renders_before = c.diag.1;
    assert_eq!(
        c.height(&its[0], &t(), true, false),
        h0,
        "高度该从备忘里读回来"
    );
    assert_eq!(c.diag.1, renders_before, "读高度不该重新渲染");
}

#[test]
fn idle_sync_after_a_width_change_still_renders() {
    // 宽度变化清掉行；下一帧若因为「什么都没变」就早退，屏幕会空着。
    let bs = convo(20);
    let its = items(&bs);
    let mut c = BlockCache::new();
    c.sync(&its, 1, 60);
    c.sync(&its, 1, 80); // 宽度变了：清行重排
    c.sync(&its, 1, 80); // 什么都没变
    let rows = c.rows_for(&its, &all(2), &t(), true, false);
    assert!(!rows.is_empty(), "宽度变化之后窗口空了");
}

/// Ctrl+T：藏掉思考。缓存为 reasoning 只存「可见」那一份，所以**出图时必须
/// 真的跳过**，否则开关是死的，还会留下空行。
#[test]
fn hidden_reasoning_paints_nothing() {
    let es = vec![
        Entry::User {
            content: "问题".into(),
        },
        Entry::Reasoning {
            content: "内心独白".into(),
        },
        Entry::Assistant {
            content: "答案".into(),
            usage: None,
        },
    ];
    let bs = flat(&es, 1);
    let its = items(&bs);
    let mut c = BlockCache::new();
    c.sync(&its, 1, 80);
    assert!(has(&c.rows_for(&its, &all(3), &t(), true, false), "内心独白"));

    c.sync(&its, 1, 80);
    let rows = c.rows_for(&its, &all(3), &t(), false, false);
    assert!(!has(&rows, "内心独白"), "Ctrl+T 藏不住思考（缓存路径）");
    // 藏起来的块不该留下任何独占的行：和「转录里从来没有这条思考」渲染出来
    // 必须一模一样。
    let without: Vec<Entry> = vec![es[0].clone(), es[2].clone()];
    let bs2 = flat(&without, 1);
    let its2 = items(&bs2);
    let mut c2 = BlockCache::new();
    c2.sync(&its2, 1, 80);
    assert_eq!(
        text(&rows),
        text(&c2.rows_for(&its2, &all(2), &t(), false, false)),
        "藏掉思考后画面与该条不存在时不一致"
    );
}

#[test]
fn content_swap_with_equal_block_count_must_not_render_stale_rows() {
    // Tree navigation / resume **replace** the whole window. Two branches
    // typically share the same block count while carrying different text —
    // the generation bump is what makes the swap visible. (The block keys
    // may even be identical: an ancestor's blocks are the same rows.)
    let old = vec![
        Entry::User {
            content: "旧用户消息".into(),
        },
        Entry::Assistant {
            content: "旧回答".into(),
            usage: None,
        },
    ];
    let new = vec![
        Entry::User {
            content: "新用户消息".into(),
        },
        Entry::Assistant {
            content: "新回答".into(),
            usage: None,
        },
    ];
    let bo = flat(&old, 1);
    let bn = flat(&new, 1);
    let io = items(&bo);
    let inn = items(&bn);
    let mut c = BlockCache::new();
    c.sync(&io, 1, 60);
    assert!(has(&c.rows_for(&io, &all(2), &t(), true, false), "旧回答"));

    // Same generation would (correctly) keep the cache; the session bumps it
    // on a wholesale swap, and that is what must clear it.
    c.sync(&inn, 2, 60);
    let r1 = c.rows_for(&inn, &all(2), &t(), true, false);
    assert!(!has(&r1, "旧回答"), "换 transcript 后不得渲染旧分支内容");
    assert!(has(&r1, "新回答"), "必须渲染新内容");
}

#[test]
fn same_generation_append_keeps_measured_heights() {
    let bs = convo(3);
    let its = items(&bs);
    let mut c = BlockCache::new();
    c.sync(&its, 1, 60);
    let _ = c.rows_for(&its, &all(3), &t(), true, false);
    assert!(c.heights[&1][0] != usize::MAX, "已测高度");

    let mut grown = bs.clone();
    grown.push((
        4,
        vec![
            Entry::User {
                content: "再来".into(),
            },
            Entry::Assistant {
                content: "好".into(),
                usage: None,
            },
        ],
    ));
    let gi = items(&grown);
    c.sync(&gi, 1, 60);
    assert!(c.heights[&1][0] != usize::MAX, "追加不得清空已测高度");
}

#[test]
fn keys_that_leave_the_window_take_their_heights() {
    let bs = convo(5);
    let its = items(&bs);
    let mut c = BlockCache::new();
    c.sync(&its, 1, 60);
    let _ = c.rows_for(&its, &all(5), &t(), true, false);
    assert!(c.heights.contains_key(&1));
    // 回退 / 窗口前移：键 1 不在窗口里了。
    c.sync(&its[1..], 1, 60);
    assert!(!c.heights.contains_key(&1), "离开窗口的键该连高度一起丢");
    assert!(c.heights.contains_key(&2));
}

#[test]
fn a_foldable_exchange_has_one_variant_when_both_states_render_alike() {
    // 短路工具输出：折叠与展开画出来一样 → 一份变体，量一次高。
    let req = Entry::ToolRequest {
        call_id: "c1".into(),
        name: "todo_write".into(),
        args: "{}".into(),
        intent: "记一笔".into(),
        text: String::new(),
        first: true,
    };
    let res = Entry::ToolResult {
        call_id: "c1".into(),
        name: "todo_write".into(),
        ok: true,
        result: "ok".into(),
        details: None,
        duration_ms: 0,
    };
    let bs = vec![(1, vec![req, res])];
    let its = items(&bs);
    let mut c = BlockCache::new();
    c.sync(&its, 1, 60);
    let folded = c.height(&its[0], &t(), true, false);
    let expanded = c.height(&its[0], &t(), true, true);
    assert_eq!(folded, expanded, "两态画得一样就该是一份变体");
    assert_eq!(c.slots[&1].variants.len(), 1);
}

#[test]
fn reasoning_is_single_variant_and_hides_cleanly() {
    let bs = vec![(
        1,
        vec![Entry::Reasoning {
            content: "想了一会儿".into(),
        }],
    )];
    let its = items(&bs);
    let mut c = BlockCache::new();
    c.sync(&its, 1, 60);
    assert!(c.height(&its[0], &t(), true, false) > 0);
    assert_eq!(
        c.height(&its[0], &t(), false, false),
        0,
        "藏起来 = 不占行"
    );
    assert!(paints(its[0].entries, true));
    assert!(!paints(its[0].entries, false));
}

/// 补色只碰给出行区间里的段（缓存是唯一的上色入口，几何一律不动）。
#[test]
fn coloring_through_the_cache_touches_only_the_given_rows() {
    let md = "```rust\nlet a = 1;\n```\n\n中间正文。\n\n```python\nx = 1\n```\n";
    let bs = flat(
        &[Entry::Assistant {
            content: md.into(),
            usage: None,
        }],
        1,
    );
    let it = items(&bs);
    let mut c = BlockCache::new();
    c.sync(&it, 0, 40);
    let all_rows = all(it.len());
    let _ = c.rows_for(&it, &all_rows, &t(), true, false);
    assert_eq!(c.pending_rows(), 2, "两个围栏 = 两段待上色");
    let h_before = c.height(&it[0], &t(), true, false);

    assert_eq!(c.color_rows(1, 0, 3, &t()), 1, "只该补上头那段");
    assert_eq!(c.pending_rows(), 1, "下面那段该还留着");
    assert_eq!(
        c.height(&it[0], &t(), true, false),
        h_before,
        "补色不许改高度"
    );
    // 再把剩下的补完：待办清空，高度仍然不变。
    let n = c.color_rows(1, 0, usize::MAX, &t());
    assert_eq!(n, 1);
    assert_eq!(c.pending_rows(), 0);
    assert_eq!(c.height(&it[0], &t(), true, false), h_before);
}
