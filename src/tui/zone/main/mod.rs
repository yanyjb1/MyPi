//! MainZone —— 历史区 + 输入区 + 保留区三子区的宿主。
//!
//! 这里只做三件事：行高仲裁、广播、把动作转发给子区。
//! 具体行为全部归子区自己：Zone 不实现任何子区的语义。

use super::{Handover, SizeChange, SubZone, TermSize, Zone};
use crate::tui::zone::main::input::ExitRequest;

// ---------------------------------------------------------------------------
// MainZone —— 三个子区槽位 + 一份仲裁结果。
// ---------------------------------------------------------------------------

/// 历史区与下面那块（输入框 + 状态栏）之间永远空一行。
///
/// 不留这一行，最后一条消息会贴着输入框的边框，读起来像被吃掉了。
/// 它是**布局事实**，所以由仲裁扣掉、渲染补上——两处必须用同一个数。
pub const HISTORY_GAP: u16 = 1;

/// 一次仲裁的结果快照，广播的单位。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Heights {
    pub history: u16,
    pub input: u16,
    pub reserved: u16,
    /// 历史区与下面那块之间留出的空行（0 = 终端太矮，让位给内容）。
    pub gap: u16,
}

/// 主区。子区以字段形式挂载；谁的行为在谁自己的模块里实现。
#[derive(Default)]
pub struct MainZone {
    /// 历史子区（订阅行高广播）。
    pub history: history::HistoryZone,
    /// 输入子区（自限行高，不订阅）。
    pub input: input::InputZone,
    /// 保留子区（声明行高的源头）。
    pub reserved: reserved::ReservedZone,
    /// APP 启动时移交的终端尺寸；resize 广播时更新。
    pub size: Option<TermSize>,
    /// 最近一次仲裁结果。
    pub heights: Heights,
    /// 子区上报的会话出口请求（input → main → app 逐级取走）。
    pub pending_exit: Option<ExitRequest>,
}

impl MainZone {
    /// 行高仲裁：子区申请（`request_height`），Zone 按规矩切分并 `assign`。
    /// 规矩：输入区自限，保留区按声明，历史区吃剩余。
    pub fn arbitrate(&mut self) -> Heights {
        let Some(size) = self.size else {
            return Heights::default();
        };
        // 输入区先申请：自限（按内容换行算，上限 1/4 终端高）。
        let input_rows = self.input.request_height(size).min(size.rows);
        self.input.assign(size, input_rows);
        // 保留区按声明。
        let reserved_rows = self.reserved.request_height(size).min(size.rows);
        self.reserved.assign(size, reserved_rows);
        // 历史区吃剩余。
        let rest = size
            .rows
            .saturating_sub(input_rows)
            .saturating_sub(reserved_rows);
        // 只有真的还有富余才留那一行：3 行的终端上空行会挤掉唯一的历史行
        // （宁可贴在一起，也不能没有历史）。留则两处一致——同一份仲裁结果。
        let gap = if rest > HISTORY_GAP { HISTORY_GAP } else { 0 };
        let history_rows = rest.saturating_sub(gap);
        self.history.assign(size, history_rows);
        let h = Heights {
            history: history_rows,
            input: input_rows,
            reserved: reserved_rows,
            gap,
        };
        self.heights = h;
        h
    }

    /// 硬件光标该落哪（终端绝对坐标，列, 行）。
    ///
    /// 谁渲染输入区谁报容器内的位置，换算成终端坐标归这里——因为只有
    /// Zone 知道三块的行高切分。钳制算术在 [`geometry::cursor_position`]。
    pub fn cursor_position(&self) -> Option<(u16, u16)> {
        let size = self.size?;
        if self.heights.input == 0 {
            return None;
        }
        Some(crate::tui::zone::main::geometry::cursor_position(
            (size.cols, size.rows),
            // 输入框实际从「历史区 + 那一行空行」开始——少算这一行，
            // 光标恒定高一格。
            self.heights.history + self.heights.gap,
            self.heights.input,
            self.heights.reserved,
            self.input.cursor_row,
            self.input.cursor_col,
        ))
    }

    /// 帧首结算 + 仲裁：子区的申请值随内容变（输入框变多行、弹窗开合），
    /// 不能只在启动/resize 时算一次。变了才广播——广播的语义是
    /// 「可用行高变了」，没变就别叫醒订阅者。
    pub fn settle(&mut self) {
        let Some(size) = self.size else {
            return;
        };
        // 保留区先结算自己的持有者与冻结行高，它是行高的声明源头。
        self.reserved.settle(size);
        let before = self.heights;
        let after = self.arbitrate();
        if after != before {
            self.broadcast();
        }
    }

    /// 行高广播：分 配结果发给每个订阅者，谁消费谁说了算。
    pub fn broadcast(&mut self) {
        let h = self.heights;
        let _ = self.history.on_height_changed(h.history);
        let _ = self.input.on_height_changed(h.input);
        let _ = self.reserved.on_height_changed(h.reserved);
    }
}

impl Zone for MainZone {
    fn attach(&mut self, size: TermSize) {
        self.size = Some(size);
        self.arbitrate();
        self.broadcast();
    }

    fn on_resize(&mut self, change: SizeChange) {
        self.size = Some(change.to);
        self.arbitrate();
        self.broadcast();
    }

    /// 事件广播：按声明拨给**每一个**声明了解释该事件的子区。
    /// 不排序、不抢跑——先后次序就是优先级，而这里没有优先级。
    /// 每个子区自己决定消费不消费；Zone 不看返回值做路由。
    fn deliver(&mut self, event: crate::tui::zone::RawEvent) -> Option<Handover> {
        use super::RawEventKind;
        let kind = match event {
            crate::tui::zone::RawEvent::Key { .. } => RawEventKind::Key,
            crate::tui::zone::RawEvent::Paste(_) => RawEventKind::Paste,
            crate::tui::zone::RawEvent::ScrollUp => RawEventKind::ScrollUp,
            crate::tui::zone::RawEvent::ScrollDown => RawEventKind::ScrollDown,
        };
        if self.history.accepts().contains(&kind) {
            self.history.deliver(&event);
        }
        if self.input.accepts().contains(&kind) {
            self.input.deliver(&event);
            // 输入区的借用包裹：收件人写死保留区，Zone 只投递不拆包。
            match self.input.pending_lend.take() {
                Some(crate::tui::zone::main::input::ToReserved::Lend(a)) => {
                    let _ = self.reserved.lend(&a);
                }
                Some(crate::tui::zone::main::input::ToReserved::Confirm) => {
                    // 确认回执：替换文本带回输入区写回编辑器。
                    let text = self.input.editor.text();
                    let cursor = self.input.editor.cursor();
                    match self.reserved.confirm(&text, cursor) {
                        Some(act) => self.input.apply_completion(act),
                        // 没有候选可确认（补全弹窗没开，或这个词已经是叶子）：
                        // 这一下 Enter 落回**提交**。`/compact` 这类无参数命令
                        // 正是叶子——精确命中让弹窗关闭，于是这里提交。
                        None => {
                            self.input.submit_now();
                        }
                    }
                }
                None => {}
            }
            // 编辑后喂补全：文本/光标/光标绝对列。补全服务没上岗时静默。
            {
                let text = self.input.editor.text();
                let cursor = self.input.editor.cursor();
                let col = self.input.cursor_col as usize;
                self.reserved.refresh_completion(&text, cursor, col);
                // 弹窗开合是**输入区**决定键位归属的依据（开着 = ↑↓/Tab/
                // Enter/Esc 归补全服务），而它只有 Zone 看得见。
                self.input
                    .set_completion_open(self.reserved.completion_open());
            }
            // 出口请求逐级上报：先到 Zone，APP 之后取走。
            if self.pending_exit.is_none() {
                self.pending_exit = self.input.pending_exit.take();
            }
        }
        if self.reserved.accepts().contains(&kind) {
            self.reserved.deliver(&event);
        }
        None
    }

    /// 自渲染：各子区画自己分到的行高。历史区需要 palette 与
    /// 可变块缓存，先画历史再画输入/保留。
    fn render(&mut self) -> Vec<ratatui::text::Line<'static>> {
        // 先结算再画：这一帧的分配结果必须反映这一帧的内容。
        self.settle();
        let p = crate::tui::theme::Palette::current();
        let ht = crate::tui::zone::main::history::render::theme::HistoryTheme::resolve();
        let term_w = self.size.map(|s| s.cols).unwrap_or(80);
        let mut out = self.history.render_rows(term_w, &ht);
        for _ in 0..self.heights.gap {
            out.push(ratatui::text::Line::default());
        }
        out.extend(self.input.render_rows(term_w, &p));
        out.extend(self.reserved.render_rows(term_w, &p));
        out
    }
}

pub mod geometry;
pub mod history;
pub mod input;
pub mod reserved;


#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::entry::Entry;
    use crate::tui::zone::TermSize;

    /// 走一遍真帧路径（和 loop.rs 的 `draw_frame` 同一套调用）：主区出行
    /// → ratatui 画进后端 → 读回缓冲。检查画出来的格子真的在缓冲里
    /// （行丢了/串行了都会挂），以及硬件光标落在容器内。
    #[test]
    fn the_frame_path_paints_through_ratatui_and_keeps_the_cursor_inside() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::widgets::Paragraph;

        let tmp = std::env::temp_dir().join(format!("mypi_frame_path_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("zeta.rs"), "x").unwrap();

        let (cols, rows) = (60u16, 20);
        let mut mz = MainZone::default();
        mz.attach(TermSize { cols, rows });
        mz.reserved.attach_completion(tmp.clone(), tmp.clone());
        for c in "cat ./ze".chars() {
            mz.deliver(crate::tui::zone::RawEvent::Key {
                key: ratatui::crossterm::event::KeyEvent {
                    code: ratatui::crossterm::event::KeyCode::Char(c),
                    modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
                    kind: ratatui::crossterm::event::KeyEventKind::Press,
                    state: ratatui::crossterm::event::KeyEventState::NONE,
                },
            });
        }
        let lines = mz.render();
        let mut term = Terminal::new(TestBackend::new(cols, rows)).unwrap();
        let mut cursor = None;
        term.draw(|f| {
            f.render_widget(Paragraph::new(lines.clone()), f.area());
            if let Some((x, y)) = mz.cursor_position() {
                f.set_cursor_position((x, y));
                cursor = Some((x, y));
            }
        })
        .unwrap();
        let buf = term.backend().buffer().clone();
        let row_text = |y: u16| -> String {
            (0..cols).map(|x| buf[(x, y)].symbol().to_string()).collect()
        };
        let screen: String = (0..rows).map(|y| row_text(y) + "\n").collect();

        // 编辑器内容、底边、弹窗候选都真的画进了缓冲
        assert!(screen.contains("cat ./ze"), "输入内容没画出来:\n{screen}");
        assert!(screen.contains("zeta.rs"), "弹窗候选没画出来:\n{screen}");
        // 容器位置**从画出来的东西反推**（不拿 heights 自己算自己）：
        // 输入框的最后一行是状态栏，往上数 input 行就是整个容器。
        let container_bottom = rows - mz.heights.reserved - 1;
        assert!(
            row_text(container_bottom).starts_with("+-"),
            "容器底边不在预期位置:\n{screen}"
        );

        let (x, y) = cursor.expect("有输入区就该有光标");
        // 光标必须落在**输入内容那一行**上。只断言"在容器范围内"抓不住
        // 差一格：容器有两行时，错一行的光标照样"在范围内"。
        let text_row = (0..rows)
            .find(|r| row_text(*r).contains("cat ./ze"))
            .expect("输入内容画在了哪一行");
        assert_eq!(
            y, text_row,
            "光标不在它那一行上：光标 {y}，内容画在 {text_row}（差一格就是行高算错了）:\n{screen}"
        );
        assert!(x < cols, "光标列越界: {x}");
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// 一帧的行数必须**恰好**等于终端高：历史 + 输入 + 保留，不多不少。
    /// 多了会被 ratatui 截掉（内容在下半屏失踪），少了下面会留灰。
    #[test]
    fn a_frame_is_exactly_terminal_height() {
        for rows in [4u16, 6, 12, 24, 40] {
            for cols in [40u16, 80, 121] {
                let mut mz = MainZone::default();
                mz.attach(TermSize { cols, rows });
                mz.history.load_plain(vec![
                    Entry::User {
                        content: "一些内容".into(),
                    },
                    Entry::Assistant {
                        content: "回答".into(),
                        usage: None,
                    },
                    Entry::ToolRequest {
                        call_id: "c".into(),
                        name: "bash".into(),
                        args: r#"{"intent":"跑","command":"ls"}"#.into(),
                        intent: "跑".into(),
                        text: String::new(),
                        first: true,
                    },
                ]);
                let frame = mz.render();
                assert_eq!(
                    frame.len(),
                    usize::from(rows),
                    "({cols}x{rows}) 一帧的行数不对：heights={:?}",
                    mz.heights
                );
                // 分配结果自洽：三块 + 那一行空行，正好等于终端高
                assert_eq!(
                    usize::from(mz.heights.history)
                        + usize::from(mz.heights.input)
                        + usize::from(mz.heights.reserved)
                        + usize::from(mz.heights.gap),
                    usize::from(rows),
                    "({cols}x{rows}) 行高分配对不上"
                );
            }
        }
    }
}
