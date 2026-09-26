//! 历史子区 —— 订阅行高广播，拥有回滚的全部私有状态。
//! 事件声明：键盘（折叠开关 Ctrl+T/Ctrl+O）+ 滚轮（回滚）。
//! 粘贴不声明——历史区不消费粘贴。
//!
//! **窗口化**：这里只留"看得见的那一段"。转录按**块**从服务端来，每块带着它在
//! 库里的 `block_id`；窗口跟着视口滑动，滑出去的块连同渲染行一起丢掉，要回来
//! 再按 id 点名取（`need_older` / `need_newer`）。所以内存跟对话长度无关，只跟
//! 窗口大小有关（`tui.preload` + `tui.renderMargin`）。
//!
//! **坐标是内容**：视口位置记的是"哪一块的第几行"（[`HistoryZone::anchor`]），
//! 不是"离底部多少行"。resize 让每一行的折行结果作废、每个块的高度都变；在
//! 行号口径下要把读者钉住，就必须知道视口下方**每一块**的高度——而窗口化之后
//! 那些块恰恰不在手上，两件事在数学上互斥。内容锚只要窗口里那几块，变宽变窄
//! 都不会跑偏（顺带：往末尾追加内容也不会推动读者，这一条不再需要专门维持）。

use super::SubZone;
use std::collections::VecDeque;

pub mod cache;
pub mod render;

use crate::server::entry::{Entry, TodoPhase};
use crate::tui::zone::main::history::cache::{BlockCache, Item};
use crate::tui::zone::main::history::render::theme::HistoryTheme;
use crate::tui::zone::{RawEvent, RawEventKind};
use ratatui::text::Line;

/// 活尾巴块的键起点：一个极大的数，恒排在所有真实块 id 之后。
///
/// 活块（还没落盘的那截尾巴）没有库里的 id，但缓存需要一个**身份**：序号不
/// 行（窗口一滑就漂），内容也不行（它一直在长）。所以按"第几块 + 这块现在有
/// 几条条目"发一个键——最后那块长出一条新条目时键就变了，缓存据此重画它，而
/// 它前面的块键不动、行照留。
const LIVE_BASE: i64 = i64::MAX - (1 << 40);

fn live_key(pos: usize, entries: usize) -> i64 {
    LIVE_BASE + (pos as i64) * 8 + entries.clamp(1, 7) as i64
}

fn is_live(key: i64) -> bool {
    key >= LIVE_BASE
}

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
    rows: Vec<Line<'static>>,
}

impl LiveRows {
    /// 量过没有？没量过时高度未知——那个 0 不能当下界用（见
    /// [`HistoryZone::render_rows`] 的判据）。
    fn measured(&self) -> bool {
        self.key.is_some()
    }

    fn materialize(&mut self, live: &LiveTail, width: usize, show_reasoning: bool, t: &HistoryTheme) {
        let key = (
            live.version,
            width,
            show_reasoning,
            crate::tui::theme::theme_epoch(),
        );
        if self.key == Some(key) {
            return;
        }
        self.rows = render::chat::live_tail(
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

/// 窗口里的一块：**键**（身份）+ 条目。
///
/// 已落盘的块键就是它的 `block_id`；活尾巴的块用 [`live_key`]。渲染缓存按这个
/// 键认行，翻页按它点名要区间。
#[derive(Clone)]
pub struct WindowBlock {
    pub key: i64,
    pub entries: Vec<Entry>,
}

/// 视口上沿锚。
///
/// **块内**是主形态：内容锚，resize / 折叠都不会让它跑偏。另一种是
/// **尾巴里**——流式那半句长过一屏时，读者往上滚，上沿就落在它里面。
/// 尾巴永远贴在显示最下面，所以那里用"离显示底部多少行"记就够了（它本来
/// 就是转瞬即逝的内容，不值得为它编 id）。
#[derive(Clone, Copy, PartialEq, Debug)]
enum Anchor {
    Block { key: i64, row: usize },
    Tail { off: usize },
}

/// 积压的滚动：行口径（滚轮/翻页）或块口径（基准里的"每次 N 块"）。
///
/// 块口径要在渲染帧才换算成行——只有那时候才知道每一块多高。两者不能相加
/// （单位不同），所以换口径时新的覆盖旧的：这是最后一次输入的意图。
#[derive(Clone, Copy)]
enum Pending {
    Rows(isize),
    Blocks(isize),
}

impl Default for Pending {
    fn default() -> Self {
        Pending::Rows(0)
    }
}

/// 聊天回滚的全部私有状态。
pub struct HistoryZone {
    /// 贴末尾跟随（false 之后视口由 `anchor` 说了算）。
    pub scroll_pinned: bool,
    /// 未跟随时视口上沿的**内容锚**：`(块键, 块内第几行)`。
    ///
    /// 记行号（"离底部多少行"）就必须知道视口下方每一块的高度，而窗口化之后
    /// 那些块不在手上；resize 又让所有高度作废。记内容就不需要它们。
    anchor: Option<Anchor>,
    /// Global reasoning fold (Ctrl+T). false = expanded by default.
    pub reasoning_folded: bool,
    /// Global tool-output expansion (Ctrl+O). false = folded per-tool thresholds.
    pub tools_expanded: bool,
    /// Zone 分配给本子区的可用行高。
    pub rows: u16,
    /// 上一帧转录实际能用的行数（扣掉任务清单）。测试与诊断用。
    last_viewport: usize,
    /// 自上次渲染以来读者往上推过（`wheel_step(up)`）。「滚到顶了」的判定
    /// 要用它：夹着不动这件事 resize 也会触发，只有「读者推过 + 这一帧被夹住」
    /// 才是真的想看更老的。
    scrolled_up: bool,
    /// 「上面/下面没有了」——服务端回了一段空的（终止符）之后闩死。
    ///
    /// 没有它，读者在两端一格一格滚就会一格一格白问服务端（一次往返 + 一次
    /// 几十毫秒的后台读）。换转录（resume / 树跳转）时重置。
    no_more_above: bool,
    no_more_below: bool,
    /// 这一帧要开口要的区间边界（一次一取，见 [`HistoryZone::take_want`]）。
    want_older: Option<i64>,
    want_newer: Option<i64>,
    /// 还没兑现的滚动（+ = 往上）。两种口径：滚轮/翻页按**行**，基准里按
    /// **块**（"每次不超过 10 个块"这种量级）。窗口那头可能还没有内容，位置
    /// 就留着，等页回来接着滚（见 `scroll_by`）。
    pending: Pending,
    /// 当前任务清单（最后一条 `Entry::Todo` 的副本）。
    ///
    /// 清单是**状态**不是叙述，所以它不进转录流，而是贴在历史区底部
    /// （见 [`HistoryZone::render_rows`]）——聊得再长也一眼看得见。
    pub todo: Vec<TodoPhase>,

    /// 已落盘的窗口：**连续**的一段，按 `block_id` 升序。
    ///
    /// 滑出去的块在下一次 `need_older` / `need_newer` 里按 id 取回来，不需要
    /// 知道它们当初在第几位。
    window: VecDeque<WindowBlock>,
    /// 活尾巴（服务端说"还没落盘"的那截）分组后的块，恒排在窗口之后。
    ///
    /// 单独放的原因：它翻不回来——库里没有。窗口可以把它甩掉（读者在上古历史
    /// 里时它在几十万块之外），但甩掉就再也取不回来，所以它不参与驱逐；与窗口
    /// 相接（窗口尾就是尾巴起点）时才上屏。
    live_blocks: Vec<WindowBlock>,
    /// 活尾巴的原始条目（服务端每次发来的那一截，整体替换）。
    live_entries: Vec<Entry>,
    /// 本前端自己产生的条目（协议错误、通知）。它们永远不进库，所以单独一列，
    /// 排在做活尾巴之后；服务端替换活尾巴时它们不动。
    local: Vec<Entry>,
    /// 已落盘的最新块 id（0 = 一块都没有）。
    tail_id: i64,
    /// 锚点上面保多少块（`tui.preload`）。
    preload: usize,
    /// 视口上下各保多少块**渲染好的行**（`tui.renderMargin`）。
    render_margin: usize,
    /// 转录代。只能由整体替换推进（见 [`HistoryZone::replace_transcript`]）。
    generation: u64,
    /// 块缓存渲染器（变体 + 高度 + 测量全在里面）。
    cache: BlockCache,
    /// 流式尾巴（还没定稿的那半句）。**不是转录**：不进窗口、不进块缓存、
    /// 不进「代」的账。
    live: LiveTail,
    /// 尾巴行的备忘（见 [`LiveRows`]）。
    live_rows: LiveRows,
}

impl Default for HistoryZone {
    fn default() -> Self {
        Self {
            scroll_pinned: true,
            anchor: None,
            reasoning_folded: false,
            tools_expanded: false,
            rows: 0,
            last_viewport: 0,
            scrolled_up: false,
            no_more_above: false,
            no_more_below: false,
            want_older: None,
            want_newer: None,
            pending: Pending::Rows(0),
            todo: Vec::new(),
            window: VecDeque::new(),
            live_blocks: Vec::new(),
            live_entries: Vec::new(),
            local: Vec::new(),
            tail_id: 0,
            preload: 128,
            render_margin: 64,
            generation: 0,
            cache: BlockCache::new_public(),
            live: LiveTail::default(),
            live_rows: LiveRows::default(),
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
    /// 只换行高，**不动视口锚**：锚是内容坐标，跟视口高没有关系。曾经这里
    /// 按 `min(rows)` 夹过一刀——那是把视口高当成了文档高，任何一次 resize
    /// 都把往上翻了几百行的用户拽回底部一屏内。
    fn on_height_changed(&mut self, rows: u16) -> bool {
        self.rows = rows;
        true
    }

    /// 声明：键盘（折叠开关）+ 滚轮（回滚）。粘贴不声明。
    fn accepts(&self) -> &'static [RawEventKind] {
        &[
            RawEventKind::Key,
            RawEventKind::ScrollUp,
            RawEventKind::ScrollDown,
        ]
    }

    fn deliver(&mut self, event: &RawEvent) -> bool {
        self.interpret(event)
    }
}

impl HistoryZone {
    /// 窗口边距（`tui.preload` / `tui.renderMargin`）。启动时配一次。
    pub fn set_window(&mut self, preload: usize, render_margin: usize) {
        self.preload = preload.max(8);
        self.render_margin = render_margin.max(8);
        self.cache.set_budget(self.render_margin);
    }

    pub fn preload(&self) -> usize {
        self.preload
    }

    pub fn render_margin(&self) -> usize {
        self.render_margin
    }

    /// 取走「要更老 / 更新的一段」：`(更新?, 边界 id, 块数)`。
    ///
    /// 历史区 → `MainZone` → `App` → 事件循环（不走 deliver 那条出口路）。
    pub fn take_want(&mut self) -> Option<(bool, i64, usize)> {
        if let Some(edge) = self.want_older.take() {
            // 一次要多一点：一整页往返比来回问便宜。
            return Some((false, edge, self.preload));
        }
        self.want_newer
            .take()
            .map(|edge| (true, edge, self.render_margin))
    }
}

impl HistoryZone {
    // ---- 服务端来的东西（块按 id 说话） ----

    /// 整体快照（服务端 `Transcript`）：窗口 = 库里的尾巴那截 + 活尾巴。
    ///
    /// 代 +1：整条转录换了（resume / 树跳转 / 补齐），缓存里的行全部作废。
    pub fn replace_transcript(
        &mut self,
        blocks: Vec<crate::server::wire::WireBlock>,
        live: Vec<Entry>,
    ) {
        self.window = blocks
            .into_iter()
            .map(|b| WindowBlock {
                key: b.id,
                entries: b.entries,
            })
            .collect();
        self.live_entries = live;
        self.local.clear();
        self.rebuild_live();
        self.tail_id = self.window.iter().map(|b| b.key).max().unwrap_or(0);
        self.todo = self.current_todo();
        self.generation = self.generation.wrapping_add(1);
        // 换了会话/分支：终止符、积压的滚动全是上一条转录的事实。
        self.no_more_above = false;
        self.no_more_below = false;
        self.want_older = None;
        self.want_newer = None;
        self.pending = Pending::Rows(0);
        self.scrolled_up = false;
        self.scroll_pinned = true;
        self.anchor = None;
        // 上一支那半句还挂在尾巴上的话，它不属于这条转录。
        self.live.clear();
    }

    /// 一段回合落盘了（服务端 `Blocks`）：追加新块 + **整段替换**活尾巴。
    ///
    /// 活尾巴是"还没落盘的那些"，落盘之后它们变成块、离开这一列；服务端知道
    /// 具体少了哪些（只有它同时看着两边），所以这里收的是替换而不是增量。
    pub fn push_blocks(&mut self, blocks: Vec<crate::server::wire::WireBlock>, live: Vec<Entry>) {
        for b in blocks {
            if !self.window.iter().any(|w| w.key == b.id) {
                self.window.push_back(WindowBlock {
                    key: b.id,
                    entries: b.entries,
                });
            }
            self.tail_id = self.tail_id.max(b.id);
        }
        self.live_entries = live;
        self.rebuild_live();
        self.todo = self.current_todo();
    }

    /// **更老的**块，前置（惰性历史）。空的一段 = 上面没有了。
    ///
    /// 收完**立刻**把锚点够不着的那些丢掉：一帧里可能连收好几页（滚轮按住
    /// 时就是这样），不在这里收，窗口会先涨到几页再等下一次渲染才缩回去——
    /// 那几页的内存就白顶成高水位了（真机实测：深滚一次顶 +20 MB）。
    pub fn prepend_blocks(&mut self, blocks: Vec<crate::server::wire::WireBlock>) {
        if blocks.is_empty() {
            self.no_more_above = true;
            return;
        }
        for b in blocks.into_iter().rev() {
            if !self.window.iter().any(|w| w.key == b.id) {
                self.window.push_front(WindowBlock {
                    key: b.id,
                    entries: b.entries,
                });
            }
        }
        self.trim_around_anchor();
    }

    /// **更新的**块（往回滚，把它曾经丢掉的区间补回来）。空 = 后面没有了。
    pub fn append_blocks(&mut self, blocks: Vec<crate::server::wire::WireBlock>) {
        if blocks.is_empty() {
            self.no_more_below = true;
            return;
        }
        for b in blocks {
            if !self.window.iter().any(|w| w.key == b.id) {
                self.window.push_back(WindowBlock {
                    key: b.id,
                    entries: b.entries,
                });
            }
            self.tail_id = self.tail_id.max(b.id);
        }
        self.trim_around_anchor();
    }

    /// 活尾巴长了几条（服务端 `Entry` / `EntryMany`，流式的热路径）。
    pub fn push_entries(&mut self, entries: Vec<Entry>) {
        self.live_entries.extend(entries);
        self.rebuild_live();
        if let Some(phases) = self.live_entries.iter().rev().find_map(|e| match e {
            Entry::Todo { phases } => Some(phases.clone()),
            _ => None,
        }) {
            self.todo = phases;
        }
    }

    /// 本前端自己造的一条（协议错误、通知）。不进库，排在活尾巴之后。
    pub fn push_local(&mut self, entry: Entry) {
        self.local.push(entry);
        self.rebuild_live();
    }

    /// 只读访问（诊断/测试）：窗口 + 活尾巴的条目。
    #[doc(hidden)]
    pub fn entries(&self) -> Vec<Entry> {
        self.window
            .iter()
            .chain(self.live_blocks.iter())
            .flat_map(|b| b.entries.clone())
            .collect()
    }

    /// 测试/预览用：把一串条目当成"已经落盘的尾巴"装进来。
    ///
    /// 分组走存储口径（[`crate::grouping::chunks`]），键按序号发——真前端拿
    /// 的是库里的 `block_id`，这里没有库，序号够用（翻页在测试里不跑）。
    #[doc(hidden)]
    pub fn load_plain(&mut self, entries: Vec<Entry>) {
        let blocks = crate::grouping::chunks(&entries)
            .into_iter()
            .enumerate()
            .map(|(i, r)| crate::server::wire::WireBlock {
                id: i as i64 + 1,
                entries: entries[r.start..r.end].to_vec(),
            })
            .collect();
        self.replace_transcript(blocks, Vec::new());
    }

    /// 已落盘的窗口有多少块（诊断/测试）。
    #[doc(hidden)]
    pub fn window_len(&self) -> usize {
        self.window.len()
    }

    /// 视口上沿的内容锚（诊断/测试）。
    #[doc(hidden)]
    pub fn top_block(&self) -> Option<(i64, usize)> {
        match self.anchor {
            Some(Anchor::Block { key, row }) => Some((key, row)),
            // 尾巴里的上沿没有块 id：报 0 号键 + 离底行数（诊断与基站在用）。
            Some(Anchor::Tail { off }) => Some((0, off)),
            None => None,
        }
    }

    /// 流式快照到达（wire 的 `stream` 消息，≤30 Hz）。
    ///
    /// 只记「还没定稿的那半句」；正式条目由 [`Self::push_entries`] 走，
    /// 两者在服务端**同一时刻**交接（见 [`LiveTail`]）。
    pub fn set_live(&mut self, reasoning: String, text: String, tool_output: String) {
        self.live.set(reasoning, text, tool_output);
    }

    /// Bench/diagnostic harness getter (doc-hidden, not API): how much the
    /// block cache is actually holding right now — `(blocks, rows)`.
    #[doc(hidden)]
    pub fn cache_stats(&self) -> (usize, usize) {
        (self.cache.cached_blocks(), self.cache.cached_rows())
    }

    /// Bench/diagnostic harness getter (doc-hidden, not API): cumulative
    /// `(block rows served from the cache, block rows that had to be
    /// rendered)`. This is the number that answers "滚动时历史在不在重渲染" —
    /// a contiguous scroll should be almost all cache hits.
    #[doc(hidden)]
    /// 窗口里还剩多少段没上色（诊断/测试用）。
    pub fn pending_highlights(&self) -> usize {
        self.cache.pending_rows()
    }

    pub fn render_counts(&self) -> (u64, u64) {
        self.cache.diag
    }

    /// Bench/diagnostic harness getter (doc-hidden, not API): the block
    /// budget in force, so a run's output cannot misreport which budget it
    /// measured.
    #[doc(hidden)]
    pub fn block_budget(&self) -> usize {
        self.cache.budget()
    }

    /// 折叠开关（Ctrl+T / Ctrl+O 的语义归本子区）。锚是内容坐标，折叠不会让
    /// 读者跑偏，只可能让它落在一个变矮的块上——渲染帧里自愈（见 `settle_anchor`）。
    pub fn toggle_reasoning(&mut self) {
        self.reasoning_folded = !self.reasoning_folded;
    }

    pub fn toggle_tools(&mut self) {
        self.tools_expanded = !self.tools_expanded;
    }

    /// 鼠标滚轮一步。Up unpin；滚回底部重新 pin。
    ///
    /// 位置**不在这一帧改**：要按块高走行，而高度只有渲染那一帧才有（窗口里的
    /// 块也不是都量过）。所以这里只记行数，[`Self::render_rows`] 兑现。
    pub fn wheel_step(&mut self, up: bool, amount: u16) {
        self.add_scroll(Pending::Rows(amount as isize), up);
    }

    /// 翻页（PageUp/PageDown、Ctrl+↑/↓）：一屏。
    pub fn page_step(&mut self, up: bool) {
        let rows = self.last_viewport.max(1) as isize;
        self.add_scroll(Pending::Rows(rows), up);
    }

    /// 按**块**滚（基准用的就是它：每次不超过 10 个块）。
    pub fn scroll_blocks(&mut self, up: bool, blocks: u16) {
        self.add_scroll(Pending::Blocks(blocks as isize), up);
    }

    /// 记一笔滚动。换方向 = 换意图：上一程夹在窗口边上没兑现的那点余量
    /// **不留**了（跟"滚过头再滚回来"一样，读者要的是回到他离开的地方）。
    fn add_scroll(&mut self, step: Pending, up: bool) {
        if up {
            self.scrolled_up = true;
            self.scroll_pinned = false;
        }
        let step = match step {
            Pending::Rows(n) => Pending::Rows(if up { n } else { -n }),
            Pending::Blocks(n) => Pending::Blocks(if up { n } else { -n }),
        };
        self.pending = match (self.pending, step) {
            // 同口径、同方向 → 累加（连着滚两格就是两格）
            (Pending::Rows(a), Pending::Rows(b)) if a == 0 || (a > 0) == (b > 0) => {
                Pending::Rows(a + b)
            }
            (Pending::Blocks(a), Pending::Blocks(b)) if a == 0 || (a > 0) == (b > 0) => {
                Pending::Blocks(a + b)
            }
            // 换口径或掉头 → 覆盖（见上：读者要的是回到他离开的地方）
            _ => step,
        };
    }

    // ---- 内部：活尾巴重排 / 当前清单 ----

    /// 活尾巴（服务端那截 + 本地的）重新分组。
    ///
    /// 分组用存储口径（[`crate::grouping::chunks`]）：一块 = 一次工具往返或一条
    /// 独立条目。与库里的分组**同一套规则**，所以活块变成真块时边界不会跳。
    fn rebuild_live(&mut self) {
        let mut all = self.live_entries.clone();
        all.extend(self.local.iter().cloned());
        self.live_blocks = crate::grouping::chunks(&all)
            .into_iter()
            .enumerate()
            .map(|(pos, r)| WindowBlock {
                key: live_key(pos, r.end - r.start),
                entries: all[r.start..r.end].to_vec(),
            })
            .collect();
    }

    fn current_todo(&self) -> Vec<TodoPhase> {
        self.window
            .iter()
            .chain(self.live_blocks.iter())
            .rev()
            .flat_map(|b| b.entries.iter())
            .find_map(|e| match e {
                Entry::Todo { phases } => Some(phases.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// 读者是不是就停在尾巴附近（决定流式那半句要不要量）。
    fn near_tail(&self) -> bool {
        match self.anchor {
            None => true,
            // 上沿在尾巴里 = 当然在盯尾巴。
            Some(Anchor::Tail { .. }) => true,
            Some(Anchor::Block { key, .. }) => {
                is_live(key) || self.window.iter().rev().take(4).any(|b| b.key == key)
            }
        }
    }
}

impl HistoryZone {
    // ---- 内部：几何 ----

    /// 自渲染：窗口那一段 + 流式尾巴 + 底部对齐。行数以 Zone 分配的为准，
    /// 内容不足时贴底补空行（聊天窗口贴底，live-tail 铁律）。
    ///
    /// 宽度是每帧传进来的参数（`MainZone.size` 是唯一真相源），不存字段：
    /// 终端一变宽，下一帧新宽度就到，`cache.sync` 自动全量失效重排。
    pub fn render_rows(&mut self, term_w: u16, t: &HistoryTheme) -> Vec<Line<'static>> {
        let width = usize::from(term_w.max(1));
        // 「读者推过」只描述这一次渲染之前的那一下滚动，渲染过就作废。
        let pushed_up = std::mem::take(&mut self.scrolled_up);
        if self.scroll_pinned {
            // 贴底 = 没有锚。`scroll_pinned` 是 pub 字段（外部把它掰回来是合法
            // 的），这里把不变量顺手补齐。
            self.anchor = None;
        }

        // 任务清单贴在历史区**底部**：它占的是本子区自己的行高，聊天窗口
        // 相应变矮（贴底的预留行数——和流式尾巴同一个道理）。上限半个
        // 视口：清单不该把对话挤没。
        let pin = render::todo::rows(&self.todo, width, t, usize::from(self.rows.max(1)) / 2);
        let viewport = usize::from(self.rows.max(1)).saturating_sub(pin.len()).max(1);
        self.last_viewport = viewport;

        // 尾巴只在**可能上屏**时才量：没量过时那个 0 不是下界而是「不知道」，
        // 所以先量一次（见 [`LiveRows::measured`]）。
        if !self.live_rows.measured() || self.scroll_pinned || self.near_tail() {
            self.live_rows
                .materialize(&self.live, width, !self.reasoning_folded, t);
        }

        // 窗口借出去干活（`Item` 借的是条目，而缓存要可写）——所以先把两块从
        // 字段里拿出来，等这一帧算完再放回去（旧代码对 `ranges` 也是这么干的）。
        let window = std::mem::take(&mut self.window);
        let live_blocks = std::mem::take(&mut self.live_blocks);
        let connected = match window.back().map(|b| b.key) {
            Some(last) => last == self.tail_id,
            None => true,
        };
        let items: Vec<Item> = window
            .iter()
            .chain(live_blocks.iter())
            .map(|b| Item {
                key: b.key,
                entries: &b.entries,
            })
            .collect();
        let order = display_order(&items);
        let view = (!self.reasoning_folded, self.tools_expanded);
        self.cache.sync(&items, self.generation, width);

        // 积压的滚动先在渲染帧里兑现（要高度的活只有这里有）。
        if !matches!(self.pending, Pending::Rows(0)) {
            let _ = self.scroll_by(&items, &order, t, view, viewport);
        }
        // 锚点自愈：块变矮 / 整块不画了，都不会把读者留在空白上。
        if let Some(Anchor::Block { key, row }) = self.anchor {
            self.anchor = self.settle_anchor(&items, &order, key, row, t, view);
        }

        let mut rows = if self.scroll_pinned {
            self.paint_pinned(&items, &order, t, view, viewport)
        } else {
            self.paint_anchored(&items, &order, t, view, viewport, connected)
        };
        rows.truncate(viewport);
        self.note_wants(&items, &order, t, viewport, pushed_up, view);

        // 视口上沿在显示顺序里的位置：窗口按它取舍（上面留 preload 块）。
        let top_pos = match self.anchor {
            Some(Anchor::Block { key, .. }) => order.iter().position(|&i| items[i].key == key),
            Some(Anchor::Tail { .. }) | None => self
                .bottom_top(&items, &order, t, view, viewport)
                .map(|(pos, _)| pos),
        };
        rows.extend(pin);
        self.window = window;
        self.live_blocks = live_blocks;
        self.trim(top_pos);
        rows
    }

    /// 流式尾巴在显示里占几行（含它与转录之间那条间隔）。
    fn tail_rows(&self, items: &[Item<'_>]) -> usize {
        self.live_rows.rows.len()
            + usize::from(!self.live_rows.rows.is_empty() && !items.is_empty())
    }

    /// 离显示底部 `off` 行的那一行 → 显示顺序位置 + 块内行。
    ///
    /// `row == h`（这块的行数）表示"这一块上面那条间隔"——间隔是真行，
    /// 视口上沿可以正好落在它上面。
    fn anchor_at(
        &mut self,
        items: &[Item<'_>],
        order: &[usize],
        t: &HistoryTheme,
        view: (bool, bool),
        tail: usize,
        off: usize,
    ) -> Option<Anchor> {
        let (show_reasoning, tools_expanded) = view;
        if off < tail {
            // 落在流式尾巴里：那一段没有块，记成"离底多少行"。
            return Some(Anchor::Tail { off });
        }
        let mut below = tail;
        for pos in (0..order.len()).rev() {
            let h = self.cache.height(&items[order[pos]], t, show_reasoning, tools_expanded);
            if h == 0 {
                continue;
            }
            if off > below + h {
                below += h + 1;
                continue;
            }
            // 块内行号：offset `below + h - 1` 是这块的第一行；`below + h` 是
            // 它上面那条间隔（也是合法的视口上沿）。
            let row = if off > below + h - 1 {
                h
            } else {
                below + h - 1 - off
            };
            // 第一块上面没有间隔：那是个不存在的行，当作显示顶部。
            return Some(Anchor::Block {
                key: items[order[pos]].key,
                row: if pos == 0 && row == h { 0 } else { row },
            });
        }
        None // 滚出窗口上沿了
    }

    /// 贴底那一窗的视口上沿（可能是块锚，也可能是"在尾巴里"）。
    fn bottom_anchor(
        &mut self,
        items: &[Item<'_>],
        order: &[usize],
        t: &HistoryTheme,
        view: (bool, bool),
        viewport: usize,
    ) -> Option<Anchor> {
        let tail = self.tail_rows(items);
        // 贴底 = 视口贴着显示的最下沿，所以它的上沿在离底 `viewport - 1` 行处。
        self.anchor_at(items, order, t, view, tail, viewport.saturating_sub(1))
    }

    /// 贴底那一窗上沿**落在哪一块**（尾巴里就报它上面第一块的最后一行）。
    ///
    /// 给"上面还差多少块"（预取）和窗口裁剪用：那两件事按**块**数，不按行。
    fn bottom_top(
        &mut self,
        items: &[Item<'_>],
        order: &[usize],
        t: &HistoryTheme,
        view: (bool, bool),
        viewport: usize,
    ) -> Option<(usize, usize)> {
        match self.bottom_anchor(items, order, t, view, viewport) {
            Some(Anchor::Block { key, row }) => {
                let pos = order.iter().position(|&i| items[i].key == key)?;
                Some((pos, row))
            }
            Some(Anchor::Tail { .. }) | None => {
                // 上沿在尾巴里：拿尾巴上面第一块的最后一行当参照。
                let (sr, te) = view;
                for pos in (0..order.len()).rev() {
                    let h = self.cache.height(&items[order[pos]], t, sr, te);
                    if h > 0 {
                        return Some((pos, h.saturating_sub(1)));
                    }
                }
                None
            }
        }
    }

    /// 锚点 → 离显示底部多少行（尾巴里的锚直接就是它）。
    fn offset_of(
        &mut self,
        items: &[Item<'_>],
        order: &[usize],
        t: &HistoryTheme,
        view: (bool, bool),
        tail: usize,
        anchor: Anchor,
    ) -> usize {
        let (show_reasoning, tools_expanded) = view;
        let Anchor::Block { key, row } = anchor else {
            let Anchor::Tail { off } = anchor else {
                unreachable!()
            };
            return off;
        };
        let Some(pos) = order.iter().position(|&i| items[i].key == key) else {
            return tail; // 锚块不在窗口里：当它在尾巴正上方（下一帧会取回来）
        };
        let h = self
            .cache
            .height(&items[order[pos]], t, show_reasoning, tools_expanded);
        // 块内：`row` 是块内行号（`row == h` 表示"这块上面那条间隔"）。
        let mut below = tail + h.saturating_sub(row);
        for &i in &order[(pos + 1).min(order.len())..] {
            let hh = self.cache.height(&items[i], t, show_reasoning, tools_expanded);
            if hh == 0 {
                continue;
            }
            below += hh + 1;
        }
        below
    }

    /// 兑现积压的滚动：从视口上沿那一行开始，按显示顺序一格一格走。
    ///
    /// 只在**窗口里**走（不量整窗：真机上那会是每帧几百块的渲染高水位），
    /// 撞到窗口上沿就停下，把没兑现的行数留下——更老的一页回来接着走。
    /// 走到贴底那一格就重新跟随。
    ///
    /// 上沿的表示法是 `(块, 行号)`，其中 `行号 == 块高` 表示"这一块**上面**
    /// 那条间隔"（间隔是真行，可以正好落在视口上沿）。于是同一块内的行序是个
    /// 环：间隔、第 0 行、第 1 行……最后一行。比较"谁在下"用 [`Self::row_ord`]
    /// 把它掰成线性。
    fn scroll_by(
        &mut self,
        items: &[Item<'_>],
        order: &[usize],
        t: &HistoryTheme,
        view: (bool, bool),
        viewport: usize,
    ) -> isize {
        let (show_reasoning, tools_expanded) = view;
        let tail = self.tail_rows(items);
        let pinned_off = viewport.saturating_sub(1);
        let bottom = self.bottom_top(items, order, t, view, viewport);
        // 上沿的**偏移**（离显示底部多少行）：贴底 = viewport-1。
        let mut off = match self.anchor {
            None => pinned_off,
            Some(a) => self.offset_of(items, order, t, view, tail, a),
        };
        // 块口径换成行口径：n 块 = n 个"当前这种块"的高度（含块间那条间隔）。
        // 基准里的"每次不超过 10 个块"就是这个量级。
        let mut left = match self.pending {
            Pending::Rows(n) => n,
            Pending::Blocks(n) => {
                let h = match self.anchor_at(items, order, t, view, tail, off) {
                    Some(Anchor::Block { key, .. }) => {
                        order.iter().position(|&i| items[i].key == key).map(|p| {
                            self.cache
                                .height(&items[order[p]], t, show_reasoning, tools_expanded)
                                + 1
                        })
                    }
                    _ => None,
                }
                .unwrap_or(1);
                n.saturating_mul(h as isize)
            }
        };
        // (1) 尾巴那一段先走完：那里只有行数，没有块。流式回复长过一屏时，
        //     读者往上滚的第一段路就在这儿——以前这里直接 `return 0`，于是
        //     "流式输出时怎么滚都不动"。
        if off < tail {
            let room = (tail - off) as isize; // 再往上这么多行就出了尾巴
            if left > 0 {
                let step = left.min(room);
                off += step as usize;
                left -= step;
            } else if left < 0 {
                let step = (-left).min(off as isize);
                off -= step as usize;
                left += step;
            }
        }
        // (2) 还留在尾巴里：上沿就是尾巴的某一行（它贴在显示最下面，记行数就够）。
        if off < tail {
            self.pending = Pending::Rows(left);
            if left <= 0 && off == 0 {
                self.scroll_pinned = true;
                self.anchor = None;
            } else {
                self.scroll_pinned = false;
                self.anchor = Some(Anchor::Tail { off });
            }
            return left;
        }
        // (3) 块空间：从"尾巴正上方那一行"开始，用既有走查。
        let (mut idx, mut row) = match self.anchor_at(items, order, t, view, tail, off) {
            Some(Anchor::Block { key, row }) => match order.iter().position(|&i| items[i].key == key) {
                Some(p) => (p, row),
                None => {
                    self.pending = Pending::Rows(0);
                    return 0;
                }
            },
            _ => {
                self.pending = Pending::Rows(0);
                return 0;
            }
        };
        while left != 0 {
            let h = self
                .cache
                .height(&items[order[idx]], t, show_reasoning, tools_expanded);
            if left > 0 {
                // 往上
                if row == h {
                    // 本块上面那条间隔 → 上一块的最后一行
                    if idx == 0 {
                        break; // 窗口上沿：剩下的留给更老那一页
                    }
                    idx -= 1;
                    let ph = self
                        .cache
                        .height(&items[order[idx]], t, show_reasoning, tools_expanded);
                    row = ph.saturating_sub(1);
                } else if row > 0 {
                    row -= 1;
                } else if h > 0 && idx > 0 {
                    row = h; // 本块第一行 → 它上面那条间隔
                } else {
                    if idx == 0 {
                        break;
                    }
                    idx -= 1;
                    let ph = self
                        .cache
                        .height(&items[order[idx]], t, show_reasoning, tools_expanded);
                    row = ph.saturating_sub(1);
                }
                left -= 1;
            } else {
                // 往下
                if row == h {
                    row = 0; // 间隔 → 本块第一行
                } else if row + 1 < h {
                    row += 1; // 块内往下
                } else if row + 1 == h && idx + 1 < order.len() {
                    // 本块最后一行 → 下面那条间隔（表示法 = "下一块上面那条"）
                    idx += 1;
                    row = self
                        .cache
                        .height(&items[order[idx]], t, show_reasoning, tools_expanded);
                } else if row + 1 == h {
                    break; // 窗口末尾：剩下的留给更新那一头
                } else {
                    break;
                }
                left += 1;
            }
        }
        // 走查停在最后一块的最后一行、却还有往下的余量：那几行是"间隔 + 尾巴"。
        if left < 0 {
            let h = self
                .cache
                .height(&items[order[idx]], t, show_reasoning, tools_expanded);
            let last_painted = (0..order.len()).rev().find(|&p| {
                self.cache
                    .height(&items[order[p]], t, show_reasoning, tools_expanded)
                    > 0
            });
            if last_painted == Some(idx) && row + 1 >= h {
                let step = (-left).min(tail as isize);
                off = tail.saturating_sub(step as usize);
                left += step;
                self.pending = Pending::Rows(left);
                if left <= 0 && off <= off.min(pinned_off) {
                    // 落回贴底那一格（或更低）：重新跟随。
                    self.scroll_pinned = true;
                    self.anchor = None;
                } else {
                    self.scroll_pinned = false;
                    self.anchor = Some(Anchor::Tail { off });
                }
                return left;
            }
        }
        self.pending = Pending::Rows(left);
        // 走到贴底那一格（或更下）：重新跟随。
        let ord = |r: usize, h: usize| -> i64 {
            if r == h {
                -1
            } else {
                r as i64
            }
        };
        let cur_h = self
            .cache
            .height(&items[order[idx]], t, show_reasoning, tools_expanded);
        let at_bottom = match bottom {
            Some((b_idx, b_row)) => {
                let bh = self
                    .cache
                    .height(&items[order[b_idx]], t, show_reasoning, tools_expanded);
                idx > b_idx
                    || (idx == b_idx && ord(row, cur_h) >= ord(b_row, bh))
            }
            None => true,
        };
        if left <= 0 && at_bottom {
            self.scroll_pinned = true;
            self.anchor = None;
        } else {
            self.scroll_pinned = false;
            self.anchor = Some(Anchor::Block {
                key: items[order[idx]].key,
                row,
            });
        }
        left
    }

    /// 锚点自愈：块变矮（resize / 折叠）就把它的第一行顶到视口顶——读者看到
    /// 的是同一块的开头；整块不画了（Ctrl+T 藏掉思考）就挪到下面第一个还占行
    /// 的块；块不在窗口里就留着不动，取数逻辑随后把它要回来。
    fn settle_anchor(
        &mut self,
        items: &[Item<'_>],
        order: &[usize],
        key: i64,
        row: usize,
        t: &HistoryTheme,
        view: (bool, bool),
    ) -> Option<Anchor> {
        let (show_reasoning, tools_expanded) = view;
        let Some(pos) = order.iter().position(|&i| items[i].key == key) else {
            return Some(Anchor::Block { key, row });
        };
        for &i in &order[pos..] {
            let h = self.cache.height(&items[i], t, show_reasoning, tools_expanded);
            if h == 0 {
                continue;
            }
            if items[i].key == key {
                return Some(Anchor::Block {
                    key,
                    row: if row > h { 0 } else { row },
                });
            }
            return Some(Anchor::Block {
                key: items[i].key,
                row: 0,
            });
        }
        Some(Anchor::Block { key, row })
    }
}

impl HistoryZone {
    /// 这一帧要画的块 → 它们在本帧**缓冲区**里各自占的行区间 + 总行数。
    ///
    /// 走查已经把高度量完了，偏移是可预测的（块间那条间隔只算一次）。有了它，
    /// 补色就能发生在**克隆行之前**——颜色与内容同一帧出，不是下一帧。
    fn buffer_layout(
        &self,
        items: &[Item<'_>],
        picked: &[(usize, usize)],
    ) -> (Vec<(i64, std::ops::Range<usize>)>, usize) {
        let mut at = 0usize;
        let mut layout = Vec::with_capacity(picked.len());
        for (n, &(i, h)) in picked.iter().enumerate() {
            if n > 0 {
                at += 1; // 块间那条间隔
            }
            layout.push((items[i].key, at..at + h));
            at += h;
        }
        (layout, at)
    }

    /// 给落在视口 `[lo, hi)`（缓冲区坐标）里的待上色段补色，返回补了几段。
    ///
    /// **只补看得见的**：一条 12 KB 的消息可能有 20 多个代码块，视口里通常
    /// 只有一两个——首帧、以及任何一帧，都不该为看不见的部分付上色的钱。
    /// 补过的段留在块缓存里，下一帧直接读。
    ///
    /// 从下往上遍历：读者盯的是底部，底部先出色。
    ///
    fn color_visible(
        &mut self,
        t: &HistoryTheme,
        layout: &[(i64, std::ops::Range<usize>)],
        lo: usize,
        hi: usize,
    ) -> usize {
        if hi <= lo {
            return 0;
        }
        let mut colored = 0usize;
        for (key, rows) in layout.iter().rev() {
            let a = lo.max(rows.start);
            let b = hi.min(rows.end);
            if a >= b {
                continue;
            }
            colored += self
                .cache
                .color_rows(*key, a - rows.start, b - rows.start, t);
        }
        colored
    }

    /// 贴底：从末尾往回攒够视口（再多攒 `render_margin` 块，滚动滑溜）。
    fn paint_pinned(
        &mut self,
        items: &[Item<'_>],
        order: &[usize],
        t: &HistoryTheme,
        view: (bool, bool),
        viewport: usize,
    ) -> Vec<Line<'static>> {
        let (show_reasoning, tools_expanded) = view;
        let tail_len = self.live_rows.rows.len();
        let tail_gap = usize::from(tail_len > 0 && !items.is_empty());
        let mut below = tail_len + tail_gap;
        // 走查比"要画的"多走 `render_margin` 块——那是**给缓存预热**（量过就进
        // 缓存了），但行**只克隆视口里那几块**：每帧把 64 块的行抄一遍是白烧，
        // 真机上就是这一笔把内存顶起来的。
        let mut picked: Vec<(usize, usize)> = Vec::new();
        let mut crossed = 0usize;
        for &i in order.iter().rev() {
            if below >= viewport && crossed >= WALK_AHEAD {
                break;
            }
            // 视口必须量准（不然算不出视口在哪）；视口之外只看备忘里有的，
            // 没量过就到此为止——别为"滚起来顺一点"替巨型卡片付排版钱。
            let h = if below < viewport {
                self.cache.height(&items[i], t, show_reasoning, tools_expanded)
            } else {
                match self
                    .cache
                    .known_height(&items[i], show_reasoning, tools_expanded)
                {
                    Some(h) => h,
                    None => break,
                }
            };
            if h == 0 {
                continue;
            }
            if below < viewport {
                picked.push((i, h));
            }
            below += h + 1;
            crossed += 1;
        }
        picked.reverse();
        // 偏移可预测 → 先补色，再克隆行：颜色和内容同一帧出。
        let (layout, blocks_total) = self.buffer_layout(items, &picked);
        let tail_len_now = self.live_rows.rows.len();
        let tail_gap_now = usize::from(tail_len_now > 0 && blocks_total > 0);
        let total = blocks_total + tail_gap_now + tail_len_now;
        let _ = self.color_visible(t, &layout, total.saturating_sub(viewport), total);
        let picked_idx: Vec<usize> = picked.iter().map(|&(i, _)| i).collect();
        let mut rows =
            self.cache
                .rows_for(items, &picked_idx, t, show_reasoning, tools_expanded);
        if tail_len > 0 {
            if !rows.is_empty() {
                rows.push(render::blocks::block_gap());
            }
            rows.extend_from_slice(&self.live_rows.rows);
        }
        // 贴底 = 画布**末尾**那一窗：上面多攒的那些行丢掉。
        let skip = rows.len().saturating_sub(viewport);
        rows.drain(..skip);
        if rows.len() < viewport {
            let pad = viewport - rows.len();
            rows.splice(..0, std::iter::repeat_n(Line::from(""), pad));
        }
        rows
    }

    /// 从**锚点**往下画：视口上沿就是锚（块 + 块内第几行）。
    ///
    /// 走查一路往前，凑够视口（再往前多画 `render_margin` 块）。走不到头 =
    /// 下面那段被窗口丢过 → 记下要哪一段（`want_newer`），这一帧先画手上有的。
    fn paint_anchored(
        &mut self,
        items: &[Item<'_>],
        order: &[usize],
        t: &HistoryTheme,
        view: (bool, bool),
        viewport: usize,
        connected: bool,
    ) -> Vec<Line<'static>> {
        let (show_reasoning, tools_expanded) = view;
        // 上沿在流式尾巴里：那一窗就是"尾巴被切掉底下 `off` 行"——尾巴贴在
        // 显示最下面，所以从贴底那一窗往上多留 `off` 行、再把最下面 `off` 行
        // 丢掉，正好是它。
        if let Some(Anchor::Tail { off }) = self.anchor {
            let mut rows = self.paint_pinned(items, order, t, view, viewport.saturating_add(off));
            let keep = rows.len().saturating_sub(off);
            rows.truncate(keep);
            rows.resize(viewport, Line::from(""));
            return rows;
        }
        let Some(Anchor::Block { key: akey, row: arow }) = self.anchor else {
            self.scroll_pinned = true;
            return self.paint_pinned(items, order, t, view, viewport);
        };
        let start = match order.iter().position(|&i| items[i].key == akey) {
            Some(p) => p,
            None => {
                // 锚块不在窗口里（刚被驱逐，或还没取回来）：先画窗口这一头，
                // 取数逻辑随后把它要回来。
                let older = items.first().is_some_and(|it| it.key > akey);
                if older {
                    self.want_older = stored_edge(items, true);
                    0
                } else {
                    self.want_newer = stored_edge(items, false);
                    return self.paint_pinned(items, order, t, view, viewport);
                }
            }
        };
        // 上沿落在**间隔**上时（`arow == h`），第一行其实是这一块上面那条
        // 间隔——它由"前一块之后的 gap"产生，所以要连前一块一起捡进来，并且
        // 丢到前一块的行数（那条 gap 正好在它后面）。
        let block_h = self.cache.height(&items[order[start]], t, show_reasoning, tools_expanded);
        let (start, arow) = if arow == block_h && start > 0 {
            let prev_h = self.cache.height(
                &items[order[start - 1]],
                t,
                show_reasoning,
                tools_expanded,
            );
            (start - 1, prev_h)
        } else {
            (start, arow)
        };
        // 走查比"要画的"多走 `render_margin` 块（缓存预热），但只克隆视口
        // 那几块的行——每帧抄 64 块的行是白烧。
        let need = viewport.saturating_add(arow);
        let mut picked: Vec<(usize, usize)> = Vec::new();
        // 已捡进来的块**真正**占多少行（块间那条间隔只算一次）。
        let mut painted_rows = 0usize;
        let mut walked = 0usize;
        let mut reached_end = false;
        for (n, &i) in order[start..].iter().enumerate() {
            if painted_rows >= need && walked >= WALK_AHEAD {
                break;
            }
            let h = if painted_rows < need {
                self.cache.height(&items[i], t, show_reasoning, tools_expanded)
            } else {
                match self
                    .cache
                    .known_height(&items[i], show_reasoning, tools_expanded)
                {
                    Some(h) => h,
                    None => break,
                }
            };
            if h == 0 {
                continue;
            }
            if painted_rows < need {
                painted_rows += h + if picked.is_empty() { 0 } else { 1 };
                picked.push((i, h));
            }
            walked += 1;
            if start + n + 1 == order.len() {
                reached_end = true;
            }
        }
        // 缓冲区的第 0 行就是锚点那一块的顶（`painted_rows` 从这里起算），
        // 所以可见窗口 = `[arow, arow + viewport)`——补色同样发生在克隆之前。
        let (layout, _) = self.buffer_layout(items, &picked);
        let _ = self.color_visible(t, &layout, arow, arow.saturating_add(viewport));
        let picked_idx: Vec<usize> = picked.iter().map(|&(i, _)| i).collect();
        let mut rows =
            self.cache
                .rows_for(items, &picked_idx, t, show_reasoning, tools_expanded);
        if reached_end && connected && !self.live_rows.rows.is_empty() {
            if !rows.is_empty() {
                rows.push(render::blocks::block_gap());
            }
            rows.extend_from_slice(&self.live_rows.rows);
        }
        let drop = arow.min(rows.len());
        rows.drain(..drop);
        rows.truncate(viewport);
        if rows.len() < viewport {
            if reached_end && start == 0 && arow == 0 {
                // 整篇就这么点：贴底（短转录不该悬在上面）。
                self.scroll_pinned = true;
                self.anchor = None;
            } else if !reached_end || !connected {
                self.want_newer = stored_edge(items, false);
            }
            // 下面那段还没回来：这一帧先补空（内容一回来就填上）。
            rows.resize(viewport, Line::from(""));
        }
        rows
    }

    /// 这一帧要不要开口要数。
    ///
    /// 对称的两条预取：锚点上面不满 `preload` 块就问更老的，下面不满
    /// `render_margin` 块就问更新的（活块翻不回来，只能这么补）。读者滚到
    /// 窗口上沿时 `above` 归零，同一条判据自然成立——不需要另立一个"滚到顶了"。
    fn note_wants(
        &mut self,
        items: &[Item<'_>],
        order: &[usize],
        t: &HistoryTheme,
        viewport: usize,
        _pushed_up: bool,
        view: (bool, bool),
    ) {
        // 视口上沿的**显示顺序位置**：锚在就按锚算，贴底就按贴底那一窗算
        // （贴底时视口上沿在窗口靠上的地方，预取要按它数，不能按窗口尾数）。
        let pos = match self.anchor {
            Some(Anchor::Block { key, .. }) => order
                .iter()
                .position(|&i| items[i].key == key)
                .unwrap_or(order.len().saturating_sub(1)),
            // 尾巴里的上沿：预取按"尾巴上面第一块"数。
            Some(Anchor::Tail { .. }) | None => self
                .bottom_top(items, order, t, view, viewport)
                .map(|(pos, _)| pos)
                .unwrap_or(order.len().saturating_sub(1)),
        };
        let above = pos;
        let below = order.len().saturating_sub(pos + 1);
        if !self.no_more_above
            && above < self.preload
            && let Some(edge) = stored_edge(items, true)
        {
            self.want_older = Some(edge);
        }
        if !self.no_more_below
            && below < self.render_margin
            && let Some(edge) = stored_edge(items, false)
            && edge < self.tail_id
        {
            self.want_newer = Some(edge);
        }
    }

    /// 收到新块之后就地把窗口裁到锚点附近（不等下一次渲染）。
    ///
    /// 一帧里可能连收好几页；不在这里收，窗口会先涨几页再缩回去，那几页就成了
    /// 常驻高水位。
    fn trim_around_anchor(&mut self) {
        let keep = self.preload + self.render_margin + SCREEN_BLOCKS;
        // 贴底、或上沿还在尾巴里：窗口尾就是尾巴，**只从前面丢**（绝不丢最新那块）。
        let key = match self.anchor {
            Some(Anchor::Block { key, .. }) => key,
            Some(Anchor::Tail { .. }) | None => {
                let front = self.window.len().saturating_sub(keep);
                for _ in 0..front {
                    self.window.pop_front();
                }
                return;
            }
        };
        let Some(pos) = self.window.iter().position(|b| b.key == key) else {
            return;
        };
        let front = pos.saturating_sub(self.preload + self.render_margin);
        for _ in 0..front {
            self.window.pop_front();
        }
        let pos = pos - front;
        while self.window.len() > pos + self.render_margin + SCREEN_BLOCKS {
            self.window.pop_back();
        }
    }

    /// 窗口随视口滑动：锚点上面留 `preload` 块，其余留 `preload + render_margin
    /// + 一屏` 块，多出来的丢掉。**活块不在这里**（它们翻不回来，见字段注释）。
    fn trim(&mut self, top_pos: Option<usize>) {
        let keep = self.preload + self.render_margin + SCREEN_BLOCKS;
        if self.anchor.is_none() {
            // 贴底：窗口尾 = 库里的尾巴。**只能从前面丢**——从后面丢就是
            // 把刚从服务端要回来的最新那块扔掉，下一帧再要一遍（真机上就是
            // 这样滚回去永远到不了底，还把内存churn 出十几 MB 高水位）。
            let front = self.window.len().saturating_sub(keep);
            for _ in 0..front {
                self.window.pop_front();
            }
            return;
        }
        // `top_pos` = 视口上沿在**显示顺序**里的位置；窗口只装已落盘的块，
        // 所以先夹进窗口的范围（活块在它后面）。
        let pos = top_pos
            .unwrap_or_else(|| self.window.len().saturating_sub(1))
            .min(self.window.len().saturating_sub(1));
        // 前头多留一截（preload + render_margin）：预取的**触发线**画在
        // preload，实际留到 preload+margin —— 差这一截就是"滚 64 块才取一次"
        // 的滞后，否则每滚几行就发一页请求。
        let front = pos.saturating_sub(self.preload + self.render_margin);
        for _ in 0..front {
            self.window.pop_front();
        }
        let pos = pos.saturating_sub(front);
        // 后面那一头：总长有上界，但**锚点上下都得留够**——锚块被挤出去就
        // 只能重新取数，读者手里的画面会先乱一帧。
        let floor = pos + self.render_margin + SCREEN_BLOCKS;
        while self.window.len() > keep && self.window.len() > floor {
            self.window.pop_back();
        }
    }

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
                    // 翻页：PageUp/PageDown 与 Ctrl+↑/↓ 同义（一屏）。
                    // `RawEvent::ScrollUp` 只来自鼠标，历史区以外没人消费它，
                    // 所以键盘这一路必须自己接上——不然窗口化的取数触发点在
                    // 没有滚轮的终端上根本够不着。
                    KeyCode::PageUp => {
                        self.page_step(true);
                        true
                    }
                    KeyCode::PageDown => {
                        self.page_step(false);
                        true
                    }
                    KeyCode::Up if ctrl => {
                        self.page_step(true);
                        true
                    }
                    KeyCode::Down if ctrl => {
                        self.page_step(false);
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

/// 一屏折算成几块（给窗口的"下面那一份"留的余量）。
const SCREEN_BLOCKS: usize = 24;

/// 走查比视口多走几块（`renderMargin` 是**缓存**的容量，不是"每帧预先排版
/// 多少块"）。多走这几块是为了滚动顺一点；再多就是替整窗付排版钱——冷启动
/// 一帧排 64 块，脏内容上实测 280 ms。
const WALK_AHEAD: usize = 8;

/// 显示顺序：置顶通知块拎到最前，其余按到达顺序。
///
/// 这就是 [`crate::grouping::blocks`] 的置顶规则，只是这里的条目已经是一块块
/// 的窗口了（分组在服务端做完，窗口按块滑动）。置顶块不在窗口里时（读者在上
/// 古历史里）自然不参与。
fn display_order(items: &[Item<'_>]) -> Vec<usize> {
    let mut pinned = Vec::new();
    let mut rest = Vec::new();
    for (i, it) in items.iter().enumerate() {
        if matches!(it.entries.first(), Some(Entry::System { pin: true, .. })) {
            pinned.push(i);
        } else {
            rest.push(i);
        }
    }
    pinned.extend(rest);
    pinned
}

/// 窗口里最老 / 最新那一块**真块**的键（活块不算：它们翻不回来）。
fn stored_edge(items: &[Item<'_>], front: bool) -> Option<i64> {
    let it = if front {
        items.iter().find(|it| !is_live(it.key))
    } else {
        items.iter().rev().find(|it| !is_live(it.key))
    };
    it.map(|it| it.key)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
