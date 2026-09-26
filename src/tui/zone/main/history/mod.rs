//! 历史子区 —— 订阅行高广播，拥有回滚的全部私有状态。
//! 事件声明：键盘（折叠开关 Ctrl+T/Ctrl+O）+ 滚轮（回滚）。
//! 粘贴不声明——历史区不消费粘贴。

use super::SubZone;

pub mod cache;
pub mod render;
use crate::tui::zone::{RawEvent, RawEventKind};

/// 流式尾巴：**还没进转录**的那半句（正文 + 思考）。
///
/// 与转录的边界由服务端保证：回合结束时 `SessionState::finalize_round`
/// 把缓冲折成正式条目（`Reasoning` / `Assistant`），**同一时刻**流式快照
/// 清空 —— 两者不会同时存在，也不会都不在。所以尾巴不需要「等正式条目
/// 落地再退场」这类同步，也不需要跟转录对账。
#[derive(Default)]
struct LiveTail {
    reasoning: String,
    text: String,
    /// 运行中的工具输出（服务端有界尾巴）。工具一结束就清空——正式结果条目
    /// 接手。
    tool_output: String,
    /// 内容真的变了才 +1；渲染备忘键在它上面（见 [`LiveRows`]）。
    version: u64,
}

impl LiveTail {
    fn set(&mut self, reasoning: String, text: String, tool_output: String) {
        // 帧是「整个缓冲区」的重发：内容没变就别作废备忘。逐字节比一遍
        // 比重新折行（markdown + 语法高亮，按字符数算钱）便宜几个数量级。
        if self.reasoning != reasoning || self.text != text || self.tool_output != tool_output {
            self.version = self.version.wrapping_add(1);
            self.reasoning = reasoning;
            self.text = text;
            self.tool_output = tool_output;
        }
    }

    fn clear(&mut self) {
        self.set(String::new(), String::new(), String::new());
    }
}

/// 尾巴行的备忘：`(版本, 宽度, 思考是否可见, 主题代)` 四元键。
///
/// 流式帧 ≤30 Hz 而来，重绘却是每批信号一次 —— 文本没变的那一帧不该重新
/// 折行。主题换代必须跟着作废：行里带着颜色。
#[derive(Default)]
struct LiveRows {
    key: Option<(u64, usize, bool, u64)>,
    rows: Vec<ratatui::text::Line<'static>>,
}

impl LiveRows {
    /// 量过没有？没量过时高度未知——那个 0 不能当下界用（见
    /// [`HistoryZone::render_rows`] 的判据）。
    fn measured(&self) -> bool {
        self.key.is_some()
    }

    fn materialize(
        &mut self,
        live: &LiveTail,
        width: usize,
        show_reasoning: bool,
        t: &crate::tui::zone::main::history::render::theme::HistoryTheme,
    ) {
        let key = (
            live.version,
            width,
            show_reasoning,
            crate::tui::theme::theme_epoch(),
        );
        if self.key == Some(key) {
            return;
        }
        self.rows = crate::tui::zone::main::history::render::chat::live_tail(
            &live.reasoning,
            &live.text,
            &live.tool_output,
            t,
            show_reasoning,
            width,
        );
        self.key = Some(key);
    }
}

/// 未跟随时用来锚住视野的记账。
///
/// 尾巴每长一行，画布就在底部多一行——「离画布底部 N 行」指向的内容因此
/// 往上跑一格：读者手没动，字在跑。记下上一帧底部那段的高度，就能算出
/// 它自己涨/缩了多少，把偏移补上同样的行数。
///
/// 转录本身动了（追加条目、整体替换）就不对账：那种位移是本来就有的一跳，
/// 不归这里管，锚点重新对齐即可。
#[derive(Default)]
struct Anchor {
    bottom: usize,
    generation: u64,
    entries_len: usize,
    primed: bool,
}

impl Anchor {
    /// 底部这段相对上一帧的变化行数；`None` = 转录本身动了，不可补偿。
    fn growth(&mut self, bottom: usize, generation: u64, entries_len: usize) -> Option<isize> {
        let prev = std::mem::replace(&mut self.bottom, bottom);
        let comparable =
            self.primed && generation == self.generation && entries_len == self.entries_len;
        self.generation = generation;
        self.entries_len = entries_len;
        self.primed = true;
        comparable.then(|| bottom as isize - prev as isize)
    }
}

/// 聊天回滚的全部私有状态。
pub struct HistoryZone {
    /// Scroll-follow: false once scrolled off the bottom, true when back at it.
    pub scroll_pinned: bool,
    /// History viewport offset (when unpinned; 0 = bottom).
    ///
    /// 「离**画布**底部的行数」——画布 = 转录 + 下面附着的流式尾巴。所以
    /// `0` 是「贴最新的字」，尾巴跟着往上顶；往上滚过尾巴的高度，尾巴就
    /// 整段落在视口下方（见 [`Self::cut_window`]）。
    pub chat_scroll: usize,
    /// Global reasoning fold (Ctrl+T). false = expanded by default.
    pub reasoning_folded: bool,
    /// Global tool-output expansion (Ctrl+O). false = folded per-tool thresholds.
    pub tools_expanded: bool,
    /// Zone 分配给本子区的可用行高。
    pub rows: u16,
    /// 当前任务清单（最后一条 `Entry::Todo` 的副本）。
    ///
    /// 清单是**状态**不是叙述，所以它不进转录流，而是贴在历史区底部
    /// （见 [`Self::render_rows`]）——聊得再长也一眼看得见。
    pub todo: Vec<crate::server::entry::TodoPhase>,

    /// 本子区私有的转录内容。
    ///
    /// **只从 [`Self::push_entry`] / [`Self::replace_transcript`] 改**：
    /// 追加不用动代（缓存按长度增量跟进），整体替换必须推进代——两支
    /// 转录可能块数、边界逐项相同，只有代能分辨，这是缓存唯一看不出来
    /// 的改动，所以不给它留直接赋值的口子。
    entries: Vec<crate::server::entry::Entry>,
    /// 转录代。只能由整体替换推进。
    generation: u64,
    /// 块缓存渲染器（窗口切割 + 变体缓存全在里面）。
    cache: crate::tui::zone::main::history::cache::BlockCache,
    /// 流式尾巴（还没定稿的那半句）。**不是转录**：不进 `entries`、
    /// 不进块缓存、不进「代」的账。
    live: LiveTail,
    /// 尾巴行的备忘（见 [`LiveRows`]）。
    live_rows: LiveRows,
    /// 未跟随时的视野锚（见 [`Anchor`]）。
    anchor: Anchor,
}

impl Default for HistoryZone {
    fn default() -> Self {
        Self {
            scroll_pinned: true,
            chat_scroll: 0,
            reasoning_folded: false,
            tools_expanded: false,
            rows: 0,
            entries: Vec::new(),
            generation: 0,
            cache: crate::tui::zone::main::history::cache::BlockCache::new_public(),
            live: LiveTail::default(),
            live_rows: LiveRows::default(),
            anchor: Anchor::default(),
            todo: Vec::new(),
        }
    }
}

impl SubZone for HistoryZone {
    /// 历史区吃剩余空间：申请"有多少要多少"。
    fn request_height(&self, _term: crate::tui::zone::TermSize) -> u16 {
        u16::MAX
    }

    fn assign(&mut self, _term: crate::tui::zone::TermSize, rows: u16) {
        self.rows = rows;
    }

    /// 历史区订阅广播。
    ///
    /// 只换行高，**不动滚动位置**：`chat_scroll` 是「离底部多少行」，
    /// 它跟视口高没有关系。曾经这里按 `min(rows)` 夹过一刀——那是把
    /// 视口高当成了文档高，result 是任何一次 resize 都把往上翻了几百
    /// 行的用户拽回底部一屏内。滚过头由 `window_from_bottom` 与渲染时
    /// 的底部对齐自然收敛（滚到顶就是滚到顶），不需要在这里夹。
    fn on_height_changed(&mut self, rows: u16) -> bool {
        self.rows = rows;
        true
    }

    /// 声明：键盘（折叠开关）+ 滚轮（回滚）。粘贴不声明。
    fn accepts(&self) -> &'static [RawEventKind] {
        &[RawEventKind::Key, RawEventKind::ScrollUp, RawEventKind::ScrollDown]
    }

    fn deliver(&mut self, event: &RawEvent) -> bool {
        self.interpret(event)
    }
}

impl HistoryZone {
    /// 追加一条转录（会话事件到一条就追一条）。
    pub fn push_entry(&mut self, entry: crate::server::entry::Entry) {
        if let crate::server::entry::Entry::Todo { phases } = &entry {
            self.todo = phases.clone();
        }
        self.entries.push(entry);
    }

    /// 整体替换转录（resume / 树跳转）：代 +1，缓存据此丢弃旧行。
    pub fn replace_transcript(&mut self, entries: Vec<crate::server::entry::Entry>) {
        // 换会话/分支：清单跟着转录走（最后一条 `Entry::Todo` 就是当前状态）。
        self.todo = entries
            .iter()
            .rev()
            .find_map(|e| match e {
                crate::server::entry::Entry::Todo { phases } => Some(phases.clone()),
                _ => None,
            })
            .unwrap_or_default();
        self.entries = entries;
        self.generation = self.generation.wrapping_add(1);
        // 换了会话/分支：上一支那半句还挂在尾巴上的话，它不属于这条转录。
        self.live.clear();
    }

    /// 流式快照到达（wire 的 `stream` 消息，≤30 Hz）。
    ///
    /// 只记「还没定稿的那半句」；正式条目由 [`Self::push_entry`] 走，
    /// 两者在服务端**同一时刻**交接（见 [`LiveTail`]）。
    pub fn set_live(&mut self, reasoning: String, text: String, tool_output: String) {
        self.live.set(reasoning, text, tool_output);
    }

    /// 只读访问（诊断/测试）。
    pub fn entries(&self) -> &[crate::server::entry::Entry] {
        &self.entries
    }

    /// Bench/diagnostic harness getter (doc-hidden, not API): how much the
    /// block cache is actually holding right now — `(blocks, rows)`.
    #[doc(hidden)]
    pub fn cache_stats(&self) -> (usize, usize) {
        (self.cache.cached_blocks(), self.cache.cached_rows())
    }

    /// Bench/diagnostic harness getter (doc-hidden, not API): the block
    /// budget in force, so a run's output cannot misreport which budget it
    /// measured.
    #[doc(hidden)]
    pub fn block_budget() -> usize {
        crate::tui::zone::main::history::cache::BLOCK_BUDGET
    }

    /// 折叠开关（Ctrl+T / Ctrl+O 的语义归本子区）。
    pub fn toggle_reasoning(&mut self) {
        self.reasoning_folded = !self.reasoning_folded;
    }

    pub fn toggle_tools(&mut self) {
        self.tools_expanded = !self.tools_expanded;
    }

    /// 鼠标滚轮一步。Up unpin；回到 0 重新 pin。
    pub fn wheel_step(&mut self, up: bool, amount: u16) {
        let n = amount as usize;
        if up {
            self.scroll_pinned = false;
            self.chat_scroll = self.chat_scroll.saturating_add(n);
        } else if self.chat_scroll > 0 {
            self.chat_scroll = self.chat_scroll.saturating_sub(n);
        }
        if self.chat_scroll == 0 {
            self.scroll_pinned = true;
        }
    }

    /// 自渲染：块缓存窗口 + 流式尾巴 + 底部对齐。行数以 Zone 分配的为准，
    /// 内容不足时顶部补空行（聊天窗口贴底，live-tail 铁律）。
    ///
    /// 宽度是每帧传进来的参数（`MainZone.size` 是唯一真相源），不存字段：
    /// 终端一变宽，下一帧新宽度就到，`cache.sync` 自动全量失效重排——
    /// 历史区不需要自己监听 resize。
    pub fn render_rows(
        &mut self,
        term_w: u16,
        t: &crate::tui::zone::main::history::render::theme::HistoryTheme,
    ) -> Vec<ratatui::text::Line<'static>> {
        let width = usize::from(term_w.max(1));
        let full = usize::from(self.rows.max(1));
        // 任务清单贴在历史区**底部**：它占的是本子区自己的行高，聊天窗口
        // 相应变矮（贴底的预留行数——和流式尾巴同一个道理）。上限半个
        // 视口：清单不该把对话挤没。
        let pin = crate::tui::zone::main::history::render::todo::rows(
            &self.todo,
            width,
            t,
            full / 2,
        );
        let viewport = full.saturating_sub(pin.len()).max(1);
        // 尾巴只在**可能上屏**时才量。流式正文只增不减（模型只在末尾追加），
        // 所以「量过的高度」永远是下界：尾巴的顶进不到「下界 + 一屏」以内，
        // 它就不可能出现在视野里——这一段白白重新折行是纯浪费，而往上翻着
        // 读正是最需要省的那一段。
        //
        // 但**没量过时那个 0 不是下界，是「不知道」**：按 0 跳过，画布就会
        // 假装没有尾巴，偏移换算整体差一个尾巴的高度——往上翻时内容错位，
        // 带底色的块（用户消息卡）会跑到不属于它的行上。所以先量一次。
        if !self.live_rows.measured()
            || self.chat_scroll < self.bottom_height().saturating_add(viewport)
        {
            self.live_rows
                .materialize(&self.live, width, !self.reasoning_folded, t);
        }
        let bottom = self.bottom_height();
        if let Some(grew) = self.anchor.growth(bottom, self.generation, self.entries.len()) {
            // 未跟随时，底部这段长高/缩矮多少行，偏移就跟着涨/落多少行
            // —— 锚住读者手里那几行，别让流式内容一路把画面顶上去。
            // 跟随（偏移 0）时不动：他要的就是看着字长出来。
            if !self.scroll_pinned {
                self.chat_scroll = if grew >= 0 {
                    self.chat_scroll.saturating_add(grew as usize)
                } else {
                    self.chat_scroll.saturating_sub(grew.unsigned_abs())
                };
                if self.chat_scroll == 0 {
                    self.scroll_pinned = true;
                }
            }
        }
        let (mut rows, total) = self.cut_window(self.chat_scroll, viewport, width, t);
        // 滚过头：走查一路走到第一块，说明画布总高已经成立，把偏移夹回
        // 「总高 - 视口」。不夹的话往上滚会滚进一片空白，而且滚回来要
        // 一格一格退——手感像卡住。画布总高要算上尾巴那一段。
        if let Some(total) = total {
            let canvas = total.saturating_add(self.bottom_height());
            let max_offset = canvas.saturating_sub(viewport);
            if self.chat_scroll > max_offset {
                self.chat_scroll = max_offset;
                if self.chat_scroll == 0 {
                    self.scroll_pinned = true;
                }
                // 夹完必须按新偏移重新切一遍：窗口是按旧偏移切的
                // （旧偏移越界时只切到第一块，画出来会短一截）。
                rows = self.cut_window(self.chat_scroll, viewport, width, t).0;
            }
        }
        rows.extend(pin);
        rows
    }

    /// 画布底部那一段的总高：流式尾巴 + 与转录之间那条间隔。
    ///
    /// [`Self::live_rows`] 在同一帧里已经由 [`Self::cut_window`] 填过。
    fn bottom_height(&self) -> usize {
        let tail = self.live_rows.rows.len();
        tail + usize::from(tail > 0 && !self.entries.is_empty())
    }

    /// 按「离画布底部的行数」切一窗，正好 `viewport` 行。
    ///
    /// 画布自下而上是：流式尾巴、转录。尾巴是**贴在底部的一段**，所以它
    /// 的行数在切窗**之前**就从视口里扣掉（切窗前预留行数那条铁律——不
    /// 预留的话满屏时流式内容永远挤不进来），扣剩的才是转录能用的行数。
    /// 视口滚到尾巴上方（`offset >= 底部段高`）时尾巴整段落在视口下方，
    /// 自然不参与——这也是「往上滚不该被流式内容拽住」的实现方式。
    ///
    /// 返回 `(行, 走到顶时的转录总高)`；第二项 `Some` 说明这一窗已经走到
    /// 转录顶部，调用方据此夹偏移。
    fn cut_window(
        &mut self,
        offset: usize,
        viewport: usize,
        width: usize,
        t: &crate::tui::zone::main::history::render::theme::HistoryTheme,
    ) -> (Vec<ratatui::text::Line<'static>>, Option<usize>) {
        let show_reasoning = !self.reasoning_folded;
        // 尾巴的行由 `render_rows` 按「可能上屏」判据量好（见那里），这里只
        // 读高度。
        let gap = usize::from(!self.live_rows.rows.is_empty() && !self.entries.is_empty());
        let bottom = gap + self.live_rows.rows.len();
        // 视口下沿落在转录里的行数，以及画布底部那段里**露出来**的下标区间
        // （`seg_hi` 是视口下沿、`seg_lo` 是视口上沿）。
        let below = offset.saturating_sub(bottom);
        let seg_lo = bottom.saturating_sub(offset.saturating_add(viewport));
        let seg_hi = bottom.saturating_sub(offset);
        let tail_visible = seg_hi - seg_lo;
        let entries_viewport = viewport - tail_visible;
        self.cache.sync(
            &self.entries,
            self.generation,
            t,
            show_reasoning,
            self.tools_expanded,
            width,
        );
        let window = self.cache.window_from_bottom(
            &self.entries,
            t,
            show_reasoning,
            self.tools_expanded,
            below,
            entries_viewport,
        );
        let mut rows = self.cache.rows_for(
            &self.entries,
            t,
            show_reasoning,
            self.tools_expanded,
            window.b0..window.b1,
        );
        // 对齐：窗口只覆盖视口所在的那几块，视口上方那截按 `skip_rows`
        // 丢掉，余下截到转录能用的行数。只做「留最后 viewport 行」是错
        // 的——窗口若一路画到转录底部，那等于永远显示文档底部，往上滚
        // 画面不动。
        rows.drain(..window.skip_rows.min(rows.len()));
        rows.truncate(entries_viewport);
        // 尾巴：画布底部那段是「间隔（若有）+ 尾巴行」，取它的可见区间。
        // `seg_lo < gap` 只可能是「这一段最顶上那条间隔露出来了」。
        if tail_visible > 0 {
            if seg_lo < gap {
                rows.push(crate::tui::zone::main::history::render::blocks::block_gap());
            }
            rows.extend_from_slice(
                &self.live_rows.rows[seg_lo.saturating_sub(gap)..seg_hi.saturating_sub(gap)],
            );
        }
        if rows.len() < viewport {
            // 内容装不下视口：贴底，上面补空。
            let pad = viewport - rows.len();
            rows.splice(..0, std::iter::repeat_n(ratatui::text::Line::from(""), pad));
        }
        (rows, window.total_rows)
    }
}

impl HistoryZone {
    /// 物理事件解释：折叠开关是本子区的语义，本子区自己解释原始键
    /// （Ctrl+T / Ctrl+O），不经过任何上级翻译。滚轮回滚同理。
    fn interpret(&mut self, event: &RawEvent) -> bool {
        use ratatui::crossterm::event::KeyCode;
        match event {
            RawEvent::Key { key } => {
                let ctrl = key
                    .modifiers
                    .contains(ratatui::crossterm::event::KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Char('t') if ctrl => {
                        self.toggle_reasoning();
                        true
                    }
                    KeyCode::Char('o') if ctrl => {
                        self.toggle_tools();
                        true
                    }
                    _ => false,
                }
            }
            RawEvent::ScrollUp => {
                self.wheel_step(true, 3);
                true
            }
            RawEvent::ScrollDown => {
                self.wheel_step(false, 3);
                true
            }
            RawEvent::Paste(_) => false, // 未声明，兜底防御
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::entry::Entry;
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

    fn rows_text(zone: &mut HistoryZone) -> String {
        rows_of(zone).join("\n")
    }

    fn rows_of(zone: &mut HistoryZone) -> Vec<String> {
        let t = crate::tui::zone::main::history::render::theme::HistoryTheme::resolve();
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

    fn zone_with(entries: Vec<Entry>) -> HistoryZone {
        let mut z = HistoryZone::default();
        z.assign(
            crate::tui::zone::TermSize { cols: 80, rows: 24 },
            20,
        );
        for e in entries {
            z.push_entry(e);
        }
        z
    }

    /// 两个开关真的改画面：Ctrl+T 藏思考，Ctrl+O 展开工具卡。
    /// 只看键位处理不算数——要一路验到画出来的行。
    #[test]
    fn ctrl_t_folds_thinking_and_ctrl_o_expands_tool_cards() {
        let output: String = (1..=8).map(|i| format!("line{i}\n")).collect();
        let mut z = zone_with(vec![
            Entry::Reasoning {
                content: "内心独白".into(),
            },
            Entry::Assistant {
                content: "答案".into(),
                usage: None,
            },
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
        assert!(before.contains("… 3 earlier lines"), "默认该折叠工具输出：{before}");

        // Ctrl+T：思考消失
        assert!(z.deliver(&ctrl('t')));
        let folded = rows_text(&mut z);
        assert!(!folded.contains("内心独白"), "Ctrl+T 没藏住思考：{folded}");
        assert!(folded.contains("答案"), "正文不该被连坐：{folded}");

        // Ctrl+O：工具输出全展开
        assert!(z.deliver(&ctrl('o')));
        let expanded = rows_text(&mut z);
        assert!(!expanded.contains("earlier lines"), "Ctrl+O 没展开：{expanded}");
        assert!(
            expanded.contains("line1") && expanded.contains("line8"),
            "展开后该给全文：{expanded}"
        );

        // 再按回去：两个开关都是可逆的
        z.deliver(&ctrl('t'));
        z.deliver(&ctrl('o'));
        let back = rows_text(&mut z);
        assert!(back.contains("内心独白") && back.contains("… 3 earlier lines"), "{back}");
    }

    /// 一份"什么都有"的虚拟转录：用户卡（中文 + 长行）、思考、markdown 正文、
    /// 工具交换（成功/失败/进行中）、diff、fetch、search、置顶通知、压缩标记。
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
        let long_bash: String = (1..=12).map(|i| format!("cargo test case_{i} ... ok
")).collect();
        vec![
            Entry::System {
                text: "—— 上下文已压缩 ——".into(),
                align: crate::server::entry::Align::Center,
                pin: false,
            },
            Entry::User {
                content: "帮我看看这个超长的一行用户消息会不会把卡片撑破：一二三四五六七八九十甲乙丙丁戊己庚辛壬癸".into(),
            },
            Entry::Reasoning {
                content: "先想想\n\n- 第一步\n- 第二步".into(),
            },
            Entry::Assistant {
                content: "# 标题\n\n正文一段，带 `inline code` 和 **粗体**。\n\n```rust\nfn main() {}\n```\n\n> 引用一行\n\n1. 列表项\n"
                    .into(),
                usage: None,
            },
            req("c1", "bash", r#"{"intent":"跑测试","command":"cargo test --lib"}"#, "跑测试"),
            res("c1", "bash", true, &long_bash),
            req("c2", "edit", r#"{"intent":"改个名字","path":"src/main.rs","old":"let hi = 1;","new":"let hello = 1;"}"#, "改个名字"),
            res("c2", "edit", true, "- let hi = 1;\n+ let hello = 1;"),
            req("c3", "bash", r#"{"intent":"不存在的命令","command":"nope"}"#, "不存在的命令"),
            res("c3", "bash", false, "bash: nope: command not found"),
            req("c4", "bash", r#"{"intent":"还在跑","command":"sleep 30"}"#, "还在跑"),
            req("c5", "fetch", r#"{"intent":"读文档","url":"https://docs.rs/ratatui/latest/ratatui/"}"#, "读文档"),
            res("c5", "fetch", true, "# ratatui\n\nA terminal UI library.\n\n## Layout\n\n- Constraint\n- Layout\n- Rect\n"),
            req("c6", "search", r#"{"intent":"找例子","query":"ratatui widget list example"}"#, "找例子"),
            res("c6", "search", true, "1. Examples · GitHub\n   https://github.com/x/y\n   widget demos\n\n2. Docs\n   https://docs.rs/z\n   API\n"),
            Entry::System {
                text: "已切换到模型 X".into(),
                align: crate::server::entry::Align::Left,
                pin: false,
            },
            Entry::Assistant {
                content: "收尾一句。".into(),
                usage: None,
            },
        ]
    }

    /// 样式自检（多轮）：一份「什么都有」的转录，在多个宽度 × 折叠/展开 ×
    /// 视口高度下渲染，逐行检查：
    ///  - **任何行都不许超过终端宽**（超了会被硬折行推成多一行，屏幕上
    ///    出现孤立黑块）；
    ///  - 卡片行（`▌` / `+-` / `| ` 开头）必须**恰好**占满终端宽，差一格
    ///    就是框破了；纯文本行（助手正文、系统注记）按内容长度就行；
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
                    for e in entries.clone() {
                        z.push_entry(e);
                    }
                    let t = crate::tui::zone::main::history::render::theme::HistoryTheme::resolve();
                    let rows = z.render_rows(term_w, &t);
                    assert_eq!(
                        rows.len(),
                        usize::from(viewport),
                        "({term_w},{viewport}) 视口没填满"
                    );
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

    /// 滚动的**精确**语义：偏移 N 的视口 == 整篇文档去掉最后 N 行之后的
    /// 末尾 viewport 行。这一条把「窗口边界 + 上下对齐」钉到行级，少一行
    /// 多一行都会挂。（空行是块之间的间隔，是真行，不能过滤掉。）
    #[test]
    fn offsets_slice_the_document_exactly() {
        let entries = synthetic_transcript();
        let t = crate::tui::zone::main::history::render::theme::HistoryTheme::resolve();
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
        // 基准：整篇画一遍（视口给到足够大），只剥掉贴底补白的前导空行。
        let mut whole = HistoryZone::default();
        whole.assign(crate::tui::zone::TermSize { cols: 60, rows: 400 }, 400);
        for e in entries.clone() {
            whole.push_entry(e);
        }
        let mut doc = text_of(whole.render_rows(60, &t));
        while doc.first().is_some_and(|l| l.trim().is_empty()) {
            doc.remove(0);
        }
        let total = doc.len();
        assert!(total > 40, "基准文档太短，测不出滚动：{total}");

        for viewport in [6usize, 12, 25] {
            for offset in [0usize, 1, 3, 7, 20, total - viewport] {
                let mut z = HistoryZone::default();
                z.assign(
                    crate::tui::zone::TermSize { cols: 60, rows: 400 },
                    viewport as u16,
                );
                for e in entries.clone() {
                    z.push_entry(e);
                }
                z.chat_scroll = offset;
                z.scroll_pinned = offset == 0;
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

    /// 有流式尾巴时，滚动语义必须同样逐行精确：偏移 N 的视口 == 合成画布
    /// （尾巴在最下、转录在上）去掉最后 N 行之后的末尾 viewport 行。
    /// 尾巴是**贴着底部的一段**，只要它的高度参与换算的地方错一行，往上翻
    /// 就会看到内容整体错位——而带底色的块（用户消息卡）错位时最显眼：
    /// 黑底会跑到不属于它的行上。
    #[test]
    fn offsets_slice_the_canvas_exactly_with_a_live_tail() {
        let t = crate::tui::zone::main::history::render::theme::HistoryTheme::resolve();
        let entries = synthetic_transcript();
        let live = "流式第一行\n流式第二行\n流式第三行\n流式第四行\n流式第五行\n流式第六行\n流式第七行\n流式第八行";
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

        // 基准：视口给到足够大，整条画布画一遍（尾巴 + 转录），剥掉贴底补白。
        let mut whole = HistoryZone::default();
        whole.assign(crate::tui::zone::TermSize { cols: 60, rows: 400 }, 400);
        for e in entries.clone() {
            whole.push_entry(e);
        }
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
                for e in entries.clone() {
                    z.push_entry(e);
                }
                z.set_live(String::new(), live.into(), String::new());
                z.chat_scroll = offset;
                z.scroll_pinned = offset == 0;
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

        // 再走一遍「量过之后跳过」的路径：先在贴底量一次（尾巴上屏），再跳
        // 到尾巴看不见的远端偏移——那一帧跳过渲染，但切片语义必须同样精确
        // （尾巴没变，量过的高度就是真高度）。
        for viewport in [6usize, 12, 25] {
            for offset in [90usize, total - viewport] {
                let mut z = HistoryZone::default();
                z.assign(
                    crate::tui::zone::TermSize { cols: 60, rows: 400 },
                    viewport as u16,
                );
                for e in entries.clone() {
                    z.push_entry(e);
                }
                z.set_live(String::new(), live.into(), String::new());
                z.scroll_pinned = true;
                let _ = z.render_rows(60, &t); // 贴底：量出高度
                z.chat_scroll = offset;
                z.scroll_pinned = false;
                let got = text_of(z.render_rows(60, &t));
                let start = total.saturating_sub(offset + viewport);
                let expect = &doc[start..(start + viewport).min(total)];
                assert_eq!(
                    got.as_slice(),
                    expect,
                    "跳过渲染那一帧：viewport={viewport} offset={offset}（start={start}）切片不对"
                );
            }
        }
    }

    /// 往上滚**画面真的动**：窗口是从底部往上走出来的（它下沿 = 转录
    /// 底部），所以偏移那几行要留在视口**下面**、上面多出来的剪掉。
    /// 只「留最后 viewport 行」的话永远显示文档底部——滚轮看着是死的。
    #[test]
    fn wheeling_up_actually_moves_the_view() {
        let mut z = zone_with(
            (0..6)
                .map(|i| Entry::User {
                    content: format!("第{i}条"),
                })
                .collect(),
        );
        z.rows = 6;
        let bottom = rows_text(&mut z);
        assert!(bottom.contains("第5条"), "贴底该看见最后一条：{bottom}");
        assert!(!bottom.contains("第0条"), "贴底不该看见第一条：{bottom}");

        // 往上一步（3 行）：最后一条移出视口，前一条进来
        z.deliver(&RawEvent::ScrollUp);
        let mid = rows_text(&mut z);
        assert!(!mid.contains("第5条"), "往上滚了却还停在底部：{mid}");
        assert!(mid.contains("第4条"), "往上滚该看见更早的内容：{mid}");

        // 一路到顶：第一条可见
        for _ in 0..8 {
            z.deliver(&RawEvent::ScrollUp);
        }
        let top = rows_text(&mut z);
        assert!(top.contains("第0条"), "到顶该看见第一条：{top}");
        assert!(!top.contains("第5条"), "到顶不该还看得见最后一条：{top}");

        // 滚回来：重新贴底
        for _ in 0..10 {
            z.deliver(&RawEvent::ScrollDown);
        }
        let back = rows_text(&mut z);
        assert!(back.contains("第5条") && z.scroll_pinned, "滚回底部该重新跟随：{back}");
    }

    /// 滚过头要收敛：往上滚一百格，画面停在文档顶部，偏移夹回
    /// 「总高 - 视口」，而不是留一个巨大的空偏移（滚回来得一格一格退）。
    #[test]
    fn overscrolling_past_the_top_clamps_to_the_document_top() {
        // 六块，总高远超视口
        let mut z = zone_with(
            (0..6)
                .map(|i| Entry::User {
                    content: format!("第{i}条"),
                })
                .collect(),
        );
        z.rows = 6; // 视口 6 行
        let _ = rows_text(&mut z); // 先量一遍（冷启动只量窗口）
        // 一路往上：远超文档
        for _ in 0..40 {
            z.deliver(&RawEvent::ScrollUp);
        }
        let top_text = rows_text(&mut z);
        let clamped = z.chat_scroll;
        assert!(
            clamped < 40 * 3,
            "偏移没有夹回文档顶，还是 {} 行",
            clamped
        );
        // 顶到顶：第 0 条必须可见，且它之上没有空白行
        assert!(top_text.contains("第0条"), "夹取后看不到第一条: {top_text}");
        assert!(
            !top_text.lines().next().unwrap_or("").trim().is_empty(),
            "文档顶上还留着空白行: {top_text:?}"
        );
        // 再往上滚一次不该再动（已经到顶）
        z.deliver(&RawEvent::ScrollUp);
        let _ = rows_text(&mut z);
        assert_eq!(z.chat_scroll, clamped, "到顶之后偏移还在涨");
    }

    /// 内容比视口还短时，往上滚应该直接回到底部并重新跟随。
    #[test]
    fn overscrolling_a_short_transcript_repins_to_bottom() {
        let mut z = zone_with(vec![Entry::User { content: "只有一条".into() }]);
        z.rows = 10;
        let _ = rows_text(&mut z);
        z.deliver(&RawEvent::ScrollUp);
        assert!(!z.scroll_pinned, "往上滚应该先脱开跟随");
        let _ = rows_text(&mut z);
        assert_eq!(z.chat_scroll, 0, "内容装得下就不该有偏移");
        assert!(z.scroll_pinned, "装得下就该重新贴底");
    }

    /// 藏起来的块不占位：开关前后，其它块的行位置不受影响。
    #[test]
    fn folding_thinking_does_not_shift_other_rows() {
        let mut z = zone_with(vec![
            Entry::User { content: "问".into() },
            Entry::Reasoning { content: "想".into() },
            Entry::Assistant { content: "答".into(), usage: None },
        ]);
        let before: Vec<String> = rows_text(&mut z).lines().map(|s| s.to_string()).collect();
        z.deliver(&ctrl('t'));
        let after: Vec<String> = rows_text(&mut z).lines().map(|s| s.to_string()).collect();
        let strip = |v: &[String]| -> Vec<String> {
            v.iter().filter(|l| !l.trim().is_empty()).cloned().collect()
        };
        assert_eq!(
            strip(&after),
            strip(&before).into_iter().filter(|l| !l.contains('想')).collect::<Vec<_>>(),
            "藏掉思考后剩下内容的顺序/内容变了"
        );
    }

    /// 流式那半句挂在转录下面，而且**不进块缓存**（缓存是转录的缓存）。
    /// Ctrl+T 同理管得到尾巴里那半截思考。
    #[test]
    fn live_tail_rides_the_bottom_without_entering_the_cache() {
        let mut z = zone_with(vec![Entry::User { content: "问一句".into() }]);
        let _ = rows_text(&mut z); // 先让缓存把转录画上
        let cached = z.cache_stats().0;

        z.set_live("先想想".into(), "答到一半".into(), String::new());
        let rows = rows_text(&mut z);
        assert!(
            last_row(&rows).contains("答到一半"),
            "贴底时尾巴该在最末一行：\n{rows}"
        );
        assert!(rows.contains("先想想"), "思考还没定稿，也该跟着出来");
        assert_eq!(z.cache_stats().0, cached, "尾巴绝不能进块缓存");

        // Ctrl+T 藏思考：尾巴里那半截跟着消失，正文留下。
        z.deliver(&ctrl('t'));
        let folded = rows_text(&mut z);
        assert!(!folded.contains("先想想"), "Ctrl+T 该藏掉尾巴的思考");
        assert!(last_row(&folded).contains("答到一半"));
    }

    /// 回合结束：服务端把流式缓冲折成正式条目（`finalize_round`），
    /// **同一时刻**清空快照。画面必须逐行一致 —— 这条链路唯一不能出的错
    /// 就是「跳一下」或「重一遍」。
    #[test]
    fn the_final_entries_replace_the_tail_line_for_line() {
        let reasoning = "先想一下这个问题";
        let text = "答案第一句\n答案第二句";
        let mut z = zone_with(vec![Entry::User { content: "问一句".into() }]);
        let _ = rows_text(&mut z);

        z.set_live(reasoning.into(), text.into(), String::new());
        let live = rows_of(&mut z);

        z.set_live(String::new(), String::new(), String::new());
        z.push_entry(Entry::Reasoning { content: reasoning.into() });
        z.push_entry(Entry::Assistant { content: text.into(), usage: None });
        let settled = rows_of(&mut z);

        assert_eq!(live, settled, "定稿前后画面必须逐行一致");
    }

    /// 追加一条 `Entry::Error`（后台失败的通知）必须画出来。
    ///
    /// 这条路径和"转录一次建好再渲染"不同：后台事件是**在已有内容之后追加**
    /// 的，而命令拒绝、压缩失败、连接中断全走这条路。测试里若只建一次转录，
    /// 这种"追加一块"的路径就没有任何东西守着。
    #[test]
    fn an_appended_error_entry_is_drawn() {
        let mut z = zone_with(vec![Entry::Assistant {
            content: "好，我记下了。".into(),
            usage: None,
        }]);
        let _ = rows_text(&mut z);
        z.push_entry(Entry::Error {
            text: "没有可压缩的历史".into(),
        });
        let rows = rows_text(&mut z);
        assert!(rows.contains("没有可压缩的历史"), "追加的错误必须上屏：\n{rows}");
    }

    /// 运行中的工具输出：纯文本（命令输出不是 markdown），只画尾巴，且
    /// 工具一结束就从尾巴上消失（正式结果条目接手）。
    #[test]
    fn a_running_tools_output_rides_the_tail_as_plain_text() {
        let mut z = zone_with(vec![Entry::User { content: "跑一下".into() }]);
        let _ = rows_text(&mut z);

        // `*` 必须原样出来：走 markdown 会被当成强调/列表，命令输出不是文档。
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

        // 调用结束：尾巴清空，正式卡片接手。
        z.set_live(String::new(), String::new(), String::new());
        let settled = rows_text(&mut z);
        assert!(!settled.contains("不是强调"), "结束后尾巴必须消失：\n{settled}");
    }

    /// 满屏时尾巴仍然看得见（切窗**之前**预留行数那条铁律），往上滚过它
    /// 的高度之后它整段落在视口下方，不参与、也不拽住画面。
    #[test]
    fn the_tail_stays_visible_on_a_full_screen_and_scrolls_away() {
        let mut entries = Vec::new();
        for i in 0..20 {
            entries.push(Entry::User { content: format!("问题 {i}") });
            entries.push(Entry::Assistant { content: format!("回答 {i}"), usage: None });
        }
        let mut z = zone_with(entries); // 转录远超 20 行的视口
        z.set_live(String::new(), "还在流的那一行".into(), String::new());

        let bottom = rows_text(&mut z);
        assert!(
            last_row(&bottom).contains("还在流的那一行"),
            "满屏时尾巴必须还在画面上：\n{bottom}"
        );

        // 尾巴占 2 行（间隔 + 一行正文）。往上滚 2 行，它整段退出视口。
        z.chat_scroll = 2;
        let scrolled = rows_text(&mut z);
        assert!(!scrolled.contains("还在流"), "滚离底部后尾巴不该还在：\n{scrolled}");
        assert!(scrolled.contains("回答 19"), "滚离底部 2 行仍在转录底部附近");
    }

    /// 往上翻着读的时候，流式内容不能把画面顶走：尾巴长高多少行，偏移
    /// 就补上多少行，读者手里那几行不动。贴底（跟随）时相反——要的就是
    /// 看着字把上面的内容顶上去。
    #[test]
    fn a_growing_tail_anchors_an_unpinned_reader_and_follows_a_pinned_one() {
        let pairs: Vec<Entry> = (0..20)
            .flat_map(|i| {
                [
                    Entry::User { content: format!("问题 {i}") },
                    Entry::Assistant { content: format!("回答 {i}"), usage: None },
                ]
            })
            .collect();

        let mut z = zone_with(pairs.clone());
        z.set_live(String::new(), "第一行".into(), String::new());
        let _ = rows_text(&mut z); // 先让锚点对上
        z.scroll_pinned = false;
        z.chat_scroll = 6;
        let before = rows_text(&mut z);

        z.set_live(String::new(), "第一行\n第二行\n第三行\n第四行".into(), String::new());
        let after = rows_text(&mut z);
        assert!(z.chat_scroll > 6, "偏移该跟着长高的尾巴一起涨：{}", z.chat_scroll);
        assert_eq!(before, after, "未跟随时画面被流式内容顶走了");

        // 跟随：偏移不动，新行把上面的内容顶上去。
        let mut z = zone_with(pairs);
        z.set_live(String::new(), "第一行".into(), String::new());
        let before = rows_text(&mut z);
        z.set_live(String::new(), "第一行\n第二行".into(), String::new());
        let after = rows_text(&mut z);
        assert_eq!(z.chat_scroll, 0, "跟随状态不该被补偿动过");
        assert_ne!(before, after, "跟随时该看着内容长出来");
    }

    /// 尾巴在视野外时**不重新出图**（那几帧没人读，白折行），画布底部也就不
    /// 动、偏移不补；等它回到「可能上屏」区才一次性量出真高度并把这段跳跃
    /// 补进偏移——所以滚回来时内容按滚动量整体平移，不跳。
    #[test]
    fn an_off_screen_tail_costs_nothing_and_materializes_without_a_jump() {
        let pairs: Vec<Entry> = (0..30)
            .flat_map(|i| {
                [
                    Entry::User { content: format!("问题 {i}") },
                    Entry::Assistant { content: format!("回答 {i}"), usage: None },
                ]
            })
            .collect();
        let mut z = zone_with(pairs); // 60 块 / 60 行，视口 20 行
        z.set_live(String::new(), "第一行".into(), String::new());
        let _ = rows_text(&mut z); // 贴底：量出高度（下界 = 间隔 + 1 行）
        z.scroll_pinned = false;

        z.chat_scroll = 40; // 往上翻到尾巴看不见
        let before = rows_text(&mut z);
        z.set_live(String::new(), "第一行\n第二行\n第三行\n第四行".into(), String::new());
        let after = rows_text(&mut z);
        assert_eq!(before, after, "视野外的尾巴改了画面（白折了行）");
        assert_eq!(z.chat_scroll, 40, "视野外的尾巴不该动偏移");

        // 往下滚 10 行，滚进「可能上屏」区：这一帧才量真高度（+3 行），
        // 同一次把 3 行补进偏移。画面必须**只**按滚动的 10 行平移。
        z.chat_scroll = 30;
        let far = rows_of(&mut z);
        z.chat_scroll = 20;
        let near = rows_of(&mut z);
        assert_eq!(z.chat_scroll, 23, "量出真高度后偏移没补上那 3 行");
        assert_eq!(far[10..], near[..10], "滚回来时内容跳了");
    }
}
