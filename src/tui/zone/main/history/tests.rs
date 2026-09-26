//! 历史区的行为测试：窗口滑动、内容锚、按需取数、折叠、流式尾巴。
//!
//! 这一层的公共面就是"喂它服务端消息 + 滚轮 + 折叠键，看它画出什么行"。
//! 所以下面几乎每条都从画出来的行读数，而不是问内部状态。

use super::*;
use crate::server::entry::Entry;
use crate::server::wire::WireBlock;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

fn ctrl(c: char) -> RawEvent {
    RawEvent::Key {
        key: KeyEvent {
            code: KeyCode::Char(c),
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        },
    }
}

fn theme() -> HistoryTheme {
    HistoryTheme::resolve()
}

fn rows_text(zone: &mut HistoryZone) -> String {
    rows_of(zone).join("\n")
}

fn rows_of(zone: &mut HistoryZone) -> Vec<String> {
    let t = theme();
    zone.render_rows(80, &t)
        .iter()
        .map(|l| {
            l.spans
                .iter()
                .map(|s| s.content.to_string())
                .collect::<String>()
        })
        .collect()
}

fn last_row(rows: &str) -> String {
    rows.lines().last().unwrap_or_default().to_string()
}

/// 一串条目当作"服务端给的尾巴"装进来（键 = 序号）。
fn zone_with(entries: Vec<Entry>) -> HistoryZone {
    let mut z = HistoryZone::default();
    z.assign(crate::tui::zone::TermSize { cols: 80, rows: 24 }, 20);
    z.load_plain(entries);
    z
}

fn user(s: &str) -> Entry {
    Entry::User {
        content: s.into(),
    }
}

fn assistant(s: &str) -> Entry {
    Entry::Assistant {
        content: s.into(),
        usage: None,
    }
}

/// 一段"回答 + 正文"，够高好滚。
fn tall(n: usize, tag: &str) -> Vec<Entry> {
    (0..n)
        .flat_map(|i| {
            [
                user(&format!("{tag} 问题 {i}")),
                assistant(&format!("{tag} 回答 {i}\n{}", "正文".repeat(8))),
            ]
        })
        .collect()
}

fn blocks_of(tag: &str, n: usize, base: i64) -> Vec<WireBlock> {
    (0..n)
        .map(|i| WireBlock {
            id: base + i as i64,
            entries: vec![user(&format!("{tag} {i}"))],
        })
        .collect()
}

/// 按需历史：读者滚到窗口上沿再往上，就该开口要更老的；还没到就不该要
/// （一开滚就发请求 = 每格一次往返）。
#[test]
fn older_blocks_are_only_asked_for_at_the_window_edge() {
    let mut z = zone_with(tall(400, "甲")); // 800 块
    z.rows = 20;
    // 冷启动：把预取填满（一直要到服务端说"上面没有了"为止）。
    let _ = rows_text(&mut z);
    let mut rounds = 0;
    while let Some((newer, edge, count)) = z.take_want() {
        assert!(!newer, "这一路只该要更老的");
        rounds += 1;
        assert!(rounds < 20, "取数循环没有收敛");
        if edge <= 1 {
            z.prepend_blocks(Vec::new()); // 到头了：终止符
            break;
        }
        z.prepend_blocks(blocks_of("更老", count, (edge - count as i64).max(1)));
        let _ = rows_text(&mut z);
    }
    // 预取满了：滚一小步不该再开口。
    z.wheel_step(true, 3);
    let _ = rows_text(&mut z);
    assert!(z.take_want().is_none(), "预取已经满了，不该再问");

    // 一路推到这个窗口的上沿：又要。
    z.wheel_step(true, 60_000);
    let _ = rows_text(&mut z);
    assert!(
        z.take_want().is_some_and(|(newer, _, _)| !newer),
        "顶到窗口上沿就该要更老的"
    );
}

/// 前置更老的块**不能动读者的位置**：锚是内容坐标，上面挂多少内容都跟它
/// 无关。动了就是"一补页画面就跳走"——这也是按需历史敢在读者正往上滚的
/// 时候补页的前提。
#[test]
fn prepending_older_blocks_does_not_move_the_reader() {
    let mut z = zone_with(tall(20, "新"));
    z.rows = 20;
    let _ = rows_text(&mut z);
    z.wheel_step(true, 9);
    let before = rows_text(&mut z);
    assert!(before.lines().count() > 1, "前提：有内容");
    let anchor = z.top_block();
    assert!(anchor.is_some(), "前提：已经离开底部");

    z.prepend_blocks(blocks_of("更老", 50, 1000));
    let after = rows_text(&mut z);
    assert_eq!(z.top_block(), anchor, "锚不该被前置动到");
    assert_eq!(after, before, "屏幕上一个字符都不该动");
}

/// 服务端用一段**空的**回答"上面没有了"：前端闩死，之后每格不再白问。
#[test]
fn an_empty_page_stops_the_asking() {
    let mut z = zone_with(tall(40, "甲"));
    z.rows = 20;
    let _ = rows_text(&mut z);
    let _ = z.take_want();
    z.wheel_step(true, 60_000);
    let _ = rows_text(&mut z);
    assert!(z.take_want().is_some(), "前提：会开口要");

    z.prepend_blocks(Vec::new()); // 终止符
    z.wheel_step(true, 60_000);
    let _ = rows_text(&mut z);
    let _ = z.take_want();
    let _ = rows_text(&mut z);
    assert!(z.take_want().is_none(), "上面没有了就不该再问");
}

/// 「上面还有没有更老的」是**每条转录各自的事实**：换会话/分支之后必须
/// 重新问，不能继承上一条转录的终止符。
#[test]
fn a_new_transcript_forgets_the_terminator() {
    let mut z = zone_with(vec![assistant("答")]);
    z.rows = 20;
    let _ = rows_text(&mut z);
    z.prepend_blocks(Vec::new());
    z.wheel_step(true, 60_000);
    let _ = rows_text(&mut z);
    let _ = z.take_want(); // 清掉第一次渲染留下的那一笔
    let _ = rows_text(&mut z);
    assert!(z.take_want().is_none(), "前提：终止符之后不再问");

    z.load_plain(vec![assistant("另一条会话")]);
    let _ = rows_text(&mut z);
    z.wheel_step(true, 60_000);
    let _ = rows_text(&mut z);
    assert!(z.take_want().is_some(), "换了转录就该重新开口要");
}

/// 往回滚：窗口另一头丢过的块按 id 点名要回来（`need_newer`）。
#[test]
fn newer_blocks_are_asked_for_when_the_window_lost_the_tail() {
    let mut z = zone_with(tall(400, "甲")); // 800 块
    z.rows = 12;
    let _ = rows_text(&mut z);
    // 往上滚一段、要一页补一页（真实往返长得就是这个样子），直到读者离最新的
    // 那一头足够远 —— 那时窗口另一头会把"最新那段"丢掉。
    for _ in 0..6 {
        z.wheel_step(true, 1200);
        let _ = rows_text(&mut z);
        let mut rounds = 0;
        while let Some((newer, edge, count)) = z.take_want() {
            assert!(!newer, "往上滚的过程中只该要更老的");
            rounds += 1;
            assert!(rounds < 20, "取数循环没有收敛");
            if edge <= 1 {
                z.prepend_blocks(Vec::new());
                break;
            }
            z.prepend_blocks(blocks_of("更老", count, (edge - count as i64).max(1)));
            let _ = rows_text(&mut z);
        }
    }
    // 往回滚一段（没到底）：窗口尾已经不是尾巴了 → 缺的那段要开口要。
    z.wheel_step(false, 900);
    let _ = rows_text(&mut z);
    let mut wants = Vec::new();
    while let Some(w) = z.take_want() {
        wants.push(w);
    }
    assert!(
        wants.iter().any(|(newer, _, _)| *newer),
        "窗口把尾巴那头丢了就该要更新的：{wants:?}"
    );
}

/// 滚回底部重新跟随（这样流式内容才会继续贴底长出来）。
#[test]
fn scrolling_back_down_re_follows_the_tail() {
    let mut z = zone_with(tall(60, "甲"));
    z.rows = 10;
    let _ = rows_text(&mut z);
    z.wheel_step(true, 60);
    let _ = rows_text(&mut z);
    assert!(!z.scroll_pinned, "往上滚该脱开跟随");

    z.wheel_step(false, 60_000);
    let _ = rows_text(&mut z);
    assert!(z.scroll_pinned, "滚回底部该重新跟随");
}

/// 窗口随读者滑动：翻过几十屏之后，手上的块数还是那一窗（不跟着对话长度
/// 长），渲染行缓存也封在预算里。
#[test]
fn the_window_stays_bounded_while_walking_ancient_history() {
    let mut z = zone_with(tall(400, "甲"));
    z.rows = 20;
    z.set_window(16, 8);
    let _ = rows_text(&mut z);
    let cap = z.preload() + z.render_margin() + 64;

    for _ in 0..30 {
        z.wheel_step(true, 60);
        let _ = rows_text(&mut z);
        // 每滚一段就补上服务端新给的（模拟取数回来）。
        let front = z.top_block().map(|(k, _)| k).unwrap_or(1);
        if let Some((false, _, _)) = z.take_want() {
            z.prepend_blocks(blocks_of("更老", 16, (front - 16).max(1)));
        }
        assert!(
            z.window_len() <= cap,
            "窗口没封住：{} 块 > {cap}",
            z.window_len()
        );
        assert!(
            z.cache_stats().0 <= z.render_margin() + 8,
            "行缓存没封住：{}",
            z.cache_stats().0
        );
    }
}

/// resize：宽度变了，**同一块还在视口顶部**（内容锚的全部意义）。
#[test]
fn a_width_change_keeps_the_same_block_at_the_top() {
    let entries = tall(40, "甲");
    let mut z = zone_with(entries);
    z.rows = 16;
    let _ = rows_text(&mut z);
    z.wheel_step(true, 40);
    let _ = rows_text(&mut z);
    let anchor = z.top_block().expect("离开底部了");
    let top_line = rows_of(&mut z).first().cloned().unwrap_or_default();

    for width in [40u16, 61, 100] {
        let t = theme();
        let rows: Vec<String> = z
            .render_rows(width, &t)
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>()
            })
            .collect();
        assert_eq!(z.top_block(), Some(anchor), "{width} 宽：锚跑了");
        // 顶上那一行本来就是空的（块间间隔 / 卡片首行）——要钉的是"同一块还在
        // 上沿"，不是"第一行有字"。整屏不能空。
        assert!(
            rows.iter().any(|l| !l.trim().is_empty()),
            "{width} 宽：整屏空了"
        );
        let _ = &top_line;
    }
}

/// 锚块变矮（折叠/变窄）时把它顶到视口第一行，而不是把读者留在空白上。
#[test]
fn a_shrunken_anchor_block_is_pulled_to_the_top_row() {
    let mut z = zone_with(vec![
        user("问"),
        Entry::Reasoning {
            content: (0..12).map(|i| format!("想 {i}\n")).collect(),
        },
        assistant("答"),
    ]);
    z.rows = 8;
    let _ = rows_text(&mut z);
    // 滚到思考块中间（锚落在它里面）。
    z.wheel_step(true, 6);
    let _ = rows_text(&mut z);
    assert!(z.top_block().is_some());

    z.toggle_reasoning(); // 思考整块藏掉
    let rows = rows_text(&mut z);
    assert!(rows.contains("答"), "藏掉思考后正文该上来：\n{rows}");
    assert!(!rows.contains("想 0"), "思考该被藏住");
}

/// 折叠锚定：上沿落在工具卡里时按 Ctrl+O，**那一块必须还在视口上沿**。
#[test]
fn folding_keeps_the_reader_on_the_same_block() {
    let mut es = Vec::new();
    for i in 0..40 {
        es.push(user(&format!("问题 {i}")));
        es.push(Entry::ToolRequest {
            call_id: format!("c{i}"),
            name: "bash".into(),
            args: format!("{{\"command\":\"cmd {i}\"}}"),
            intent: "跑一下".into(),
            text: String::new(),
            first: true,
        });
        es.push(Entry::ToolResult {
            call_id: format!("c{i}"),
            name: "bash".into(),
            ok: true,
            result: format!("输出 {i}\n{}", "行".repeat(200)),
            details: None,
            duration_ms: 0,
        });
        es.push(assistant(&format!("回答 {i}")));
    }
    let mut z = zone_with(es);
    z.rows = 24;
    let _ = rows_text(&mut z);
    // 往上滚，直到锚块是个工具卡（不依赖具体行高）。
    let mut found = false;
    for _ in 0..400 {
        z.wheel_step(true, 3);
        let _ = rows_text(&mut z);
        if let Some((key, _)) = z.top_block()
            && let Some(b) = z.window.iter().find(|b| b.key == key)
            && matches!(b.entries.first(), Some(Entry::ToolRequest { .. }))
        {
            found = true;
            break;
        }
    }
    assert!(found, "找不到'锚是工具卡'的位置");
    let before = z.top_block();
    z.deliver(&ctrl('o'));
    assert_eq!(z.top_block(), before, "折叠把读者甩到别的块上了");
}

/// 两个开关真的改画面：Ctrl+T 藏思考，Ctrl+O 展开工具卡。
#[test]
fn ctrl_t_folds_thinking_and_ctrl_o_expands_tool_cards() {
    let output: String = (1..=8).map(|i| format!("line{i}\n")).collect();
    let mut z = zone_with(vec![
        Entry::Reasoning {
            content: "内心独白".into(),
        },
        assistant("答案"),
        Entry::ToolRequest {
            call_id: "c1".into(),
            name: "bash".into(),
            args: r#"{"intent":"跑","command":"echo"}"#.into(),
            intent: "跑".into(),
            text: String::new(),
            first: true,
        },
        Entry::ToolResult {
            call_id: "c1".into(),
            name: "bash".into(),
            ok: true,
            result: output,
            details: None,
            duration_ms: 0,
        },
    ]);

    let before = rows_text(&mut z);
    assert!(before.contains("内心独白"), "默认该显示思考：{before}");
    assert!(
        before.contains("… 3 earlier lines"),
        "默认该折叠工具输出：{before}"
    );

    // Ctrl+T：思考消失
    assert!(z.deliver(&ctrl('t')));
    let folded = rows_text(&mut z);
    assert!(!folded.contains("内心独白"), "Ctrl+T 没藏住思考：{folded}");
    assert!(folded.contains("答案"), "正文不该被连坐：{folded}");

    // Ctrl+O：工具输出全展开
    assert!(z.deliver(&ctrl('o')));
    let expanded = rows_text(&mut z);
    assert!(
        !expanded.contains("earlier lines"),
        "Ctrl+O 没展开：{expanded}"
    );
    assert!(
        expanded.contains("line1") && expanded.contains("line8"),
        "展开后该给全文：{expanded}"
    );

    // 再按回去：两个开关都是可逆的
    z.deliver(&ctrl('t'));
    z.deliver(&ctrl('o'));
    let back = rows_text(&mut z);
    assert!(
        back.contains("内心独白") && back.contains("… 3 earlier lines"),
        "{back}"
    );
}

/// 往上滚**画面真的动**，滚回底部重新跟随。
#[test]
fn wheeling_up_actually_moves_the_view() {
    let mut z = zone_with(
        (0..6)
            .map(|i| user(&format!("第{i}条")))
            .collect(),
    );
    z.rows = 6;
    let bottom = rows_text(&mut z);
    assert!(bottom.contains("第5条"), "贴底该看见最后一条：{bottom}");
    assert!(!bottom.contains("第0条"), "贴底不该看见第一条：{bottom}");

    z.deliver(&RawEvent::ScrollUp);
    let mid = rows_text(&mut z);
    assert!(!mid.contains("第5条"), "往上滚了却还停在底部：{mid}");
    assert!(mid.contains("第4条"), "往上滚该看见更早的内容：{mid}");

    for _ in 0..8 {
        z.deliver(&RawEvent::ScrollUp);
    }
    let top = rows_text(&mut z);
    assert!(top.contains("第0条"), "到顶该看见第一条：{top}");

    for _ in 0..10 {
        z.deliver(&RawEvent::ScrollDown);
    }
    let back = rows_text(&mut z);
    assert!(
        back.contains("第5条") && z.scroll_pinned,
        "滚回底部该重新跟随：{back}"
    );
}

/// 滚过头要收敛：一路往上，画面停在文档顶部，再往上滚也不动。
#[test]
fn overscrolling_past_the_top_stops_at_the_document_top() {
    let mut z = zone_with(
        (0..6)
            .map(|i| user(&format!("第{i}条")))
            .collect(),
    );
    z.rows = 6;
    let _ = rows_text(&mut z);
    for _ in 0..40 {
        z.deliver(&RawEvent::ScrollUp);
    }
    let top_text = rows_text(&mut z);
    let anchor = z.top_block();
    assert!(top_text.contains("第0条"), "夹取后看不到第一条: {top_text}");
    assert!(
        !top_text.lines().next().unwrap_or("").trim().is_empty(),
        "文档顶上还留着空白行: {top_text:?}"
    );

    z.deliver(&RawEvent::ScrollUp);
    let _ = rows_text(&mut z);
    assert_eq!(z.top_block(), anchor, "到顶之后锚还在动");
}

/// 内容比视口还短时，往上滚应该直接回到底部并重新跟随。
#[test]
fn overscrolling_a_short_transcript_repins_to_bottom() {
    let mut z = zone_with(vec![user("只有一条")]);
    z.rows = 10;
    let _ = rows_text(&mut z);
    z.deliver(&RawEvent::ScrollUp);
    let _ = rows_text(&mut z);
    assert!(z.scroll_pinned, "装得下就该重新贴底");
    assert!(rows_text(&mut z).contains("只有一条"));
}

/// 藏起来的块不占位：开关前后，其它块的行位置不受影响。
#[test]
fn folding_thinking_does_not_shift_other_rows() {
    let mut z = zone_with(vec![
        user("问"),
        Entry::Reasoning {
            content: "想".into(),
        },
        assistant("答"),
    ]);
    let before: Vec<String> = rows_text(&mut z).lines().map(|s| s.to_string()).collect();
    z.deliver(&ctrl('t'));
    let after: Vec<String> = rows_text(&mut z).lines().map(|s| s.to_string()).collect();
    let strip = |v: &[String]| -> Vec<String> {
        v.iter().filter(|l| !l.trim().is_empty()).cloned().collect()
    };
    assert_eq!(
        strip(&after),
        strip(&before)
            .into_iter()
            .filter(|l| !l.contains('想'))
            .collect::<Vec<_>>(),
        "藏掉思考后剩下内容的顺序/内容变了"
    );
}

/// 流式那半句挂在转录下面，而且**不进块缓存**（缓存是转录的缓存）。
#[test]
fn live_tail_rides_the_bottom_without_entering_the_cache() {
    let mut z = zone_with(vec![user("问一句")]);
    let _ = rows_text(&mut z);
    let cached = z.cache_stats().0;

    z.set_live("先想想".into(), "答到一半".into(), String::new());
    let rows = rows_text(&mut z);
    assert!(
        last_row(&rows).contains("答到一半"),
        "贴底时尾巴该在最末一行：\n{rows}"
    );
    assert!(rows.contains("先想想"), "思考还没定稿，也该跟着出来");
    assert_eq!(z.cache_stats().0, cached, "尾巴绝不能进块缓存");

    z.deliver(&ctrl('t'));
    let folded = rows_text(&mut z);
    assert!(!folded.contains("先想想"), "Ctrl+T 该藏掉尾巴的思考");
    assert!(last_row(&folded).contains("答到一半"));
}

/// 回合结束：服务端把流式缓冲折成正式条目，**同一时刻**清空快照。画面必须
/// 逐行一致——这条链路唯一不能出的错就是「跳一下」或「重一遍」。
#[test]
fn the_final_entries_replace_the_tail_line_for_line() {
    let reasoning = "先想一下这个问题";
    let text = "答案第一句\n答案第二句";
    let mut z = zone_with(vec![user("问一句")]);
    let _ = rows_text(&mut z);

    z.set_live(reasoning.into(), text.into(), String::new());
    let live = rows_of(&mut z);

    z.set_live(String::new(), String::new(), String::new());
    z.push_blocks(
        vec![
            WireBlock {
                id: 9,
                entries: vec![Entry::Reasoning {
                    content: reasoning.into(),
                }],
            },
            WireBlock {
                id: 10,
                entries: vec![assistant(text)],
            },
        ],
        Vec::new(),
    );
    let settled = rows_of(&mut z);

    assert_eq!(live, settled, "定稿前后画面必须逐行一致");
}

/// 追加一条错误条目必须画出来（前台/后台两条路都算）。
#[test]
fn an_appended_error_entry_is_drawn() {
    let mut z = zone_with(vec![assistant("好，我记下了。")]);
    let _ = rows_text(&mut z);
    z.push_entries(vec![Entry::Error {
        text: "没有可压缩的历史".into(),
    }]);
    let rows = rows_text(&mut z);
    assert!(
        rows.contains("没有可压缩的历史"),
        "服务端追加的错误必须上屏：\n{rows}"
    );

    // 本前端自己造的那条（协议错误）：不进库，也得画。
    z.push_local(Entry::Error {
        text: "[Internal] 本地通知".into(),
    });
    let rows = rows_text(&mut z);
    assert!(rows.contains("本地通知"), "本地条目必须上屏：\n{rows}");
}

/// 运行中的工具输出：纯文本（命令输出不是 markdown），只画尾巴。
#[test]
fn a_running_tools_output_rides_the_tail_as_plain_text() {
    let mut z = zone_with(vec![user("跑一下")]);
    let _ = rows_text(&mut z);

    let output: String = (1..=20).map(|i| format!("第{i}行 *不是强调*\n")).collect();
    z.set_live(String::new(), String::new(), output);
    let rows = rows_text(&mut z);
    assert!(rows.contains("*不是强调*"), "工具输出必须走纯文本：\n{rows}");
    assert!(rows.contains("第20行"), "要画的是尾巴：\n{rows}");
    assert!(
        !rows.contains("第1行"),
        "20 行输出只画最后 {} 行：\n{rows}",
        crate::tui::zone::main::history::render::chat::LIVE_TOOL_LINES
    );

    z.set_live(String::new(), String::new(), String::new());
    let settled = rows_text(&mut z);
    assert!(
        !settled.contains("不是强调"),
        "结束后尾巴必须消失：\n{settled}"
    );
}

/// 满屏时尾巴仍然看得见，往上滚过它的高度之后它整段落在视口下方。
#[test]
fn the_tail_stays_visible_on_a_full_screen_and_scrolls_away() {
    let mut z = zone_with(tall(20, "甲"));
    z.set_live(String::new(), "还在流的那一行".into(), String::new());

    let bottom = rows_text(&mut z);
    assert!(
        last_row(&bottom).contains("还在流的那一行"),
        "满屏时尾巴必须还在画面上：\n{bottom}"
    );

    z.wheel_step(true, 3);
    let scrolled = rows_text(&mut z);
    assert!(
        !scrolled.contains("还在流"),
        "滚离底部后尾巴不该还在：\n{scrolled}"
    );
}

/// 往上翻着读时，流式内容不能把画面顶走（内容锚让这条**结构上不可能错**，
/// 不需要任何补偿）；贴底时相反——要的就是看着字长出来。
#[test]
fn a_growing_tail_never_moves_an_unpinned_reader() {
    let mut z = zone_with(tall(20, "甲"));
    z.set_live(String::new(), "第一行".into(), String::new());
    let _ = rows_text(&mut z);
    z.wheel_step(true, 6);
    let before = rows_text(&mut z);
    let anchor = z.top_block();

    z.set_live(
        String::new(),
        "第一行\n第二行\n第三行\n第四行".into(),
        String::new(),
    );
    let after = rows_text(&mut z);
    assert_eq!(z.top_block(), anchor, "尾巴长高把锚顶走了");
    assert_eq!(before, after, "未跟随时画面被流式内容顶走了");

    // 跟随：新行把上面的内容顶上去，画面变。
    let mut z = zone_with(tall(20, "甲"));
    z.set_live(String::new(), "第一行".into(), String::new());
    let before = rows_text(&mut z);
    z.set_live(String::new(), "第一行\n第二行".into(), String::new());
    let after = rows_text(&mut z);
    assert!(z.scroll_pinned);
    assert_ne!(before, after, "跟随时该看着内容长出来");
}

/// 滚动的**精确**语义：底下一窗 == 整篇画布的最后 viewport 行；往上滚 N 行
/// == 画布去掉最后 N 行之后的末尾 viewport 行。少一行多一行都会挂。
#[test]
fn the_viewport_slices_the_canvas_exactly() {
    let entries = synthetic_transcript();
    let t = theme();
    let text_of = |rows: Vec<ratatui::text::Line<'static>>| -> Vec<String> {
        rows.iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>()
            })
            .collect()
    };
    let live = "流式第一行\n流式第二行\n流式第三行\n流式第四行";

    // 基准：视口给到足够大，整条画布画一遍，剥掉贴底补白。
    let mut whole = HistoryZone::default();
    whole.assign(crate::tui::zone::TermSize { cols: 60, rows: 400 }, 400);
    whole.load_plain(entries.clone());
    whole.set_live(String::new(), live.into(), String::new());
    let mut doc = text_of(whole.render_rows(60, &t));
    while doc.first().is_some_and(|l| l.trim().is_empty()) {
        doc.remove(0);
    }
    let total = doc.len();
    assert!(total > 40, "基准画布太短，测不出滚动：{total}");

    for viewport in [6usize, 12, 25] {
        for offset in [0usize, 1, 3, 7, 20, total - viewport] {
            let mut z = HistoryZone::default();
            z.assign(
                crate::tui::zone::TermSize { cols: 60, rows: 400 },
                viewport as u16,
            );
            z.load_plain(entries.clone());
            z.set_live(String::new(), live.into(), String::new());
            let _ = z.render_rows(60, &t); // 先贴底量一次（尾巴的高度）
            if offset > 0 {
                z.wheel_step(true, offset as u16);
            }
            let got = text_of(z.render_rows(60, &t));
            let start = total.saturating_sub(offset + viewport);
            let expect = &doc[start..(start + viewport).min(total)];
            assert_eq!(
                got.as_slice(),
                expect,
                "viewport={viewport} offset={offset}（start={start}）切片不对"
            );
        }
    }
}

/// 「什么都有」的一份虚拟转录（用户卡、思考、markdown、工具交换、置顶通知、
/// 压缩标记……），给切片与样式检查当素材。
fn synthetic_transcript() -> Vec<Entry> {
    let req = |id: &str, name: &str, args: &str, intent: &str| Entry::ToolRequest {
        call_id: id.into(),
        name: name.into(),
        args: args.into(),
        intent: intent.into(),
        text: String::new(),
        first: true,
    };
    let res = |id: &str, name: &str, ok: bool, result: &str| Entry::ToolResult {
        call_id: id.into(),
        name: name.into(),
        ok,
        result: result.into(),
        details: None,
        duration_ms: 0,
    };
    let long_bash: String = (1..=12).map(|i| format!("cargo test case_{i} ... ok\n")).collect();
    vec![
        Entry::System {
            text: "—— 上下文已压缩 ——".into(),
            align: crate::server::entry::Align::Center,
            pin: false,
        },
        user("帮我看看这个超长的一行用户消息会不会把卡片撑破：一二三四五六七八九十甲乙丙丁戊己庚辛壬癸"),
        Entry::Reasoning {
            content: "先想想\n\n- 第一步\n- 第二步".into(),
        },
        assistant(
            "# 标题\n\n正文一段，带 `inline code` 和 **粗体**。\n\n```rust\nfn main() {}\n```\n\n> 引用一行\n\n1. 列表项\n",
        ),
        req("c1", "bash", r#"{"intent":"跑测试","command":"cargo test --lib"}"#, "跑测试"),
        res("c1", "bash", true, &long_bash),
        req(
            "c2",
            "edit",
            r#"{"intent":"改个名字","path":"src/main.rs","old":"let hi = 1;","new":"let hello = 1;"}"#,
            "改个名字",
        ),
        res("c2", "edit", true, "- let hi = 1;\n+ let hello = 1;"),
        req("c3", "bash", r#"{"intent":"不存在的命令","command":"nope"}"#, "不存在的命令"),
        res("c3", "bash", false, "bash: nope: command not found"),
        req("c4", "bash", r#"{"intent":"还在跑","command":"sleep 30"}"#, "还在跑"),
        req(
            "c5",
            "fetch",
            r#"{"intent":"读文档","url":"https://docs.rs/ratatui/latest/ratatui/"}"#,
            "读文档",
        ),
        res("c5", "fetch", true, "# ratatui\n\nA terminal UI library.\n\n## Layout\n\n- Constraint\n- Layout\n- Rect\n"),
        req("c6", "search", r#"{"intent":"找例子","query":"ratatui widget list example"}"#, "找例子"),
        res("c6", "search", true, "1. Examples · GitHub\n   https://github.com/x/y\n   widget demos\n\n2. Docs\n   https://docs.rs/z\n   API\n"),
        Entry::System {
            text: "已切换到模型 X".into(),
            align: crate::server::entry::Align::Left,
            pin: false,
        },
        assistant("收尾一句。"),
    ]
}

/// 样式自检（多轮）：多个宽度 × 折叠状态 × 视口高度下渲染，逐行检查：
///  - **任何行都不许超过终端宽**；
///  - 卡片行（`▌` / `+-` / `| ` 开头）必须**恰好**占满终端宽；
///  - 视口恰好填满分配到的行数。
#[test]
fn style_sweep_holds_all_render_invariants() {
    use unicode_width::UnicodeWidthStr;
    let entries = synthetic_transcript();
    for term_w in [40u16, 60, 80, 121] {
        for (reasoning_folded, tools_expanded) in
            [(false, false), (true, false), (false, true), (true, true)]
        {
            for viewport in [4u16, 12, 30, 60] {
                let mut z = HistoryZone {
                    reasoning_folded,
                    tools_expanded,
                    ..Default::default()
                };
                z.assign(crate::tui::zone::TermSize { cols: term_w, rows: 24 }, viewport);
                z.load_plain(entries.clone());
                let t = theme();
                let rows = z.render_rows(term_w, &t);
                assert_eq!(
                    rows.len(),
                    usize::from(viewport),
                    "({term_w},{viewport}) 视口没填满"
                );
                // 滚上去再查一遍：滚动那一屏的行宽同样不许差一格（差一格就是
                // 终端上留一个残字，而且只有滚动时才出现）。
                z.wheel_step(true, 7);
                let scrolled = z.render_rows(term_w, &t);
                assert_eq!(scrolled.len(), usize::from(viewport), "滚动后视口没填满");
                for (i, l) in scrolled.iter().enumerate() {
                    let content: String =
                        l.spans.iter().map(|s| s.content.to_string()).collect();
                    let w: usize = l.spans.iter().map(|s| s.content.width()).sum();
                    assert!(
                        w <= term_w as usize,
                        "({term_w},{viewport}) 滚动后第 {i} 行超宽 {w}: {content:?}"
                    );
                    let framed = content.starts_with("+-")
                        || content.starts_with("| ")
                        || content.starts_with('▌');
                    assert!(
                        !framed || w == term_w as usize,
                        "({term_w},{viewport}) 滚动后第 {i} 行是卡片行却没占满（{w}）: {content:?}"
                    );
                }
                for (i, l) in rows.iter().enumerate() {
                    let content: String =
                        l.spans.iter().map(|s| s.content.to_string()).collect();
                    let w: usize = l.spans.iter().map(|s| s.content.width()).sum();
                    assert!(
                        w <= term_w as usize,
                        "({term_w},{viewport}) 第 {i} 行超宽 {w}: {content:?}"
                    );
                    let framed = content.starts_with("+-")
                        || content.starts_with("| ")
                        || content.starts_with('▌');
                    if framed {
                        assert_eq!(
                            w, term_w as usize,
                            "({term_w},{viewport}) 第 {i} 行是卡片行却没占满: {content:?}"
                        );
                    }
                }
            }
        }
    }
}

/// 窗口边距可配，但会被钳到安全值（0 会让窗口空转，和"关掉窗口化"一个意思）。
#[test]
fn window_margins_are_clamped() {
    let mut z = HistoryZone::default();
    z.set_window(0, 0);
    assert!(z.preload() >= 8 && z.render_margin() >= 8, "边距该有下限");
    z.set_window(4096, 32);
    assert_eq!(z.preload(), 4096);
    assert_eq!(z.render_margin(), 32);
    assert_eq!(z.block_budget(), 32, "行缓存预算跟着 renderMargin 走");
}

/// 任务清单：它贴在底部、占自己的行高，所以视口比分配到的行数矮；清单内容
/// 是**状态**（最后一条 `Todo` 说了算），而且窗口滚走了也不会丢。
#[test]
fn the_todo_pins_to_the_bottom_and_survives_the_window_sliding_away() {
    let todo = |n: usize| Entry::Todo {
        phases: vec![crate::server::entry::TodoPhase {
            name: "阶段".into(),
            tasks: (0..n)
                .map(|i| crate::server::entry::TodoTask {
                    content: format!("任务 {i}"),
                    status: crate::server::entry::TodoStatus::Pending,
                    blocker: None,
                })
                .collect(),
        }],
    };
    let mut z = zone_with(tall(30, "甲"));
    z.rows = 12;
    z.set_window(4, 4);
    z.push_entries(vec![todo(2)]);
    let rows = rows_text(&mut z);
    assert!(rows.contains("任务 0"), "清单该贴在底部：\n{rows}");
    let lines: Vec<&str> = rows.lines().collect();
    let todo_at = lines.iter().position(|l| l.contains("任务 0")).expect("清单在第几行");
    assert!(
        todo_at >= lines.len() - 6,
        "清单该占底部那几行，实际在第 {todo_at} 行：\n{rows}"
    );
    let _ = &lines;

    // 滚到很老的地方：清单还在（它是状态，不是这条窗口的内容）。
    z.wheel_step(true, 300);
    let rows = rows_text(&mut z);
    assert!(rows.contains("任务 0"), "滚走之后清单不见了：\n{rows}");
}

/// 翻页键：PageUp/PageDown 与 Ctrl+↑/↓ 同义，一次一屏；没有滚轮的终端也够得着
/// 窗口化的取数触发点。
#[test]
fn page_keys_scroll_a_screen() {
    let page = |code: KeyCode| RawEvent::Key {
        key: KeyEvent {
            code,
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        },
    };
    let mut z = zone_with(tall(60, "甲"));
    z.rows = 10;
    let bottom = rows_text(&mut z);
    assert!(bottom.contains("甲 问题 59"), "前提：贴底看见最新：\n{bottom}");

    // PageUp：一屏往上
    z.deliver(&RawEvent::Key {
        key: KeyEvent {
            code: KeyCode::PageUp,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        },
    });
    let up = rows_text(&mut z);
    assert!(!up.contains("甲 问题 59"), "PageUp 没动：\n{up}");
    assert!(!z.scroll_pinned, "PageUp 该脱开跟随");

    // Ctrl+↑ 同义：再往上
    z.deliver(&page(KeyCode::Up));
    let up2 = rows_text(&mut z);
    assert_ne!(up2, up, "Ctrl+↑ 没动");

    // 一路向下：回到贴底。
    for _ in 0..40 {
        z.deliver(&RawEvent::Key {
            key: KeyEvent {
                code: KeyCode::PageDown,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
        });
    }
    let back = rows_text(&mut z);
    assert!(z.scroll_pinned, "PageDown 到底该重新跟随：\n{back}");
    assert!(back.contains("甲 问题 59"), "回到最新那一头：\n{back}");
}

/// 块口径的滚动（基准用的那个）：滚 N 块 ≈ 走过 N 块内容。
#[test]
fn a_block_step_moves_about_that_many_blocks() {
    let mut z = zone_with(tall(60, "甲"));
    z.rows = 10;
    let _ = rows_text(&mut z);
    let index_of = |rows: &str| -> i64 {
        rows.lines()
            .find_map(|l| l.split("问题 ").nth(1))
            .and_then(|t| t.split_whitespace().next())
            .and_then(|n| n.trim().parse::<i64>().ok())
            .expect("屏幕上第一条问题的序号")
    };
    let before = index_of(&rows_text(&mut z));

    z.scroll_blocks(true, 10);
    let after = index_of(&rows_text(&mut z));
    let moved = before - after;
    assert!(
        (3..=30).contains(&moved),
        "滚 10 块该走过十几块上下，实际 {moved}"
    );
    assert!(!z.scroll_pinned);
}

/// 流式那半句（尾巴）长过一屏时，往上滚**必须动**。
///
/// 这是真机上"流式输出时按键/滑动都不行"的根：尾巴里的上沿以前表示不出来，
/// 一遇到就整帧放弃滚动。现在尾巴里的上沿记成"离底多少行"，往上滚先走尾巴、
/// 再进转录，滚回来还能重新贴底。
#[test]
fn scrolling_inside_a_long_live_tail_works() {
    let mut z = zone_with(tall(40, "甲"));
    z.rows = 10;
    let _ = rows_text(&mut z);
    let live: String = (0..30).map(|i| format!("流式第{i}行\n")).collect();
    z.set_live(String::new(), live, String::new());
    let bottom = rows_text(&mut z);
    assert!(bottom.contains("流式第29行"), "贴底看最新一行：\n{bottom}");

    z.wheel_step(true, 10);
    let up = rows_text(&mut z);
    assert_ne!(up, bottom, "流式期间往上滚，画面必须动");
    assert!(
        up.contains("流式第1") || up.contains("流式第0"),
        "该看见更早的流式行：\n{up}"
    );

    // 再往上：出尾巴、进转录。
    z.wheel_step(true, 20);
    let up2 = rows_text(&mut z);
    assert!(up2.contains("甲 回答 39"), "再往上该进到转录：\n{up2}");

    // 滚回来：重新贴底、尾巴又完整贴上。
    z.wheel_step(false, 200);
    let back = rows_text(&mut z);
    assert!(z.scroll_pinned, "滚回底部该重新跟随：\n{back}");
    assert!(back.contains("流式第29行"), "回来该看见最新的流式行：\n{back}");
}

/// 首帧只给**看得见**的段上色：一条长消息里有几十个围栏，视口里通常只有
/// 一两个——剩下的滚到再补（补过的留在缓存，下一帧直接读）。
#[test]
fn the_first_frame_colors_only_what_the_viewport_shows() {
    let md: String = (0..8)
        .map(|i| format!("正文第 {i} 段。\n\n```rust\nlet x{i} = {i};\n```\n\n"))
        .collect();
    let mut z = zone_with(vec![Entry::Assistant {
        content: md,
        usage: None,
    }]);
    z.rows = 10;
    let rows = z.render_rows(80, &theme());
    assert!(
        z.pending_highlights() > 0,
        "视口之外那些段不该在首帧就被上色"
    );
    // 视口里出现过高亮色（keyword 色）——看得见的那部分确实色了。
    let key = theme().get(crate::tui::zone::main::history::render::theme::Token::SyntaxKeyword);
    let painted = rows
        .iter()
        .flat_map(|l| l.spans.iter())
        .any(|s| s.style.fg == Some(key));
    assert!(painted, "视口里的那段该被上色：{rows:?}");
}
