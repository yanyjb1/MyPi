//! 输入子区 —— 自限行高，不订阅行高广播。
//!
//! 事件解释全在本子区：`deliver` 收原始 RawEvent，先看 hang，再
//! `semantics::translate` 出动作，驱动 Editor。补全解释权后续由
//! hang 状态接管（↑↓Tab 在 hang=true 时借给保留区所有者）。

use ratatui::crossterm::event::{KeyCode, KeyModifiers};

use super::SubZone;
use crate::tui::zone::RawEvent;

pub mod editor;
pub mod render;
pub mod statusline;

use editor::{Editor, Effect, History};
use statusline::{StatusEvent, StatusLine, StatusTheme};
use crate::tui::zone::main::geometry;

/// 编辑器联动的上游请求。输入区对保留区的唯一输出通道；
/// Zone 收到后投递，不认识任何键。
pub enum ToReserved {
    /// 把翻译好的动作借给保留区当前所有者。
    Lend(crate::tui::zone::main::input::semantics::Action),
    /// 确认候选（Tab/Enter，hang 时输入区的解释）：保留区服务推进
    /// 补全并返回替换，由 Zone 带回输入区写回编辑器。
    Confirm,
}

/// 会话层出口请求：编辑器不解释这些动作的"之后"，只声明"要出去"。
/// 去向（提交给谁、中断什么、退不退程序）是 Zone/APP/会话层的事。
/// 两下 Esc 的窗口。**300ms**：人手连着敲两下就在这个量级，再放宽会把
/// "随手按两下"也算成"我要看节点树"。
pub const ESC_TREE_WINDOW: std::time::Duration = std::time::Duration::from_millis(300);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitRequest {
    /// 提交当前输入（携带全文）。
    Submit(String),
    /// 中断在途回复。
    Interrupt,
    /// 空闲时在 [`ESC_TREE_WINDOW`] 之内按了两下 Esc = 开节点树。
    ///
    /// **窗口在输入区里数**，不在会话层：Esc 的归属（面板 / 补全 / hang /
    /// 打断 / 空闲）只有输入区知道，会话层拿到一个"按了一下"是没法判断
    /// 那一下该不该计入的。详见 [`InputZone::route_esc`]。
    OpenTree,
    /// 退出程序。
    Quit,
}

/// 输入子区。行高由自己的内容决定，广播来了不理。
pub struct InputZone {
    /// Input viewport start (wrapped row). Independent of the cursor.
    pub scroll: usize,
    /// 自己声明的行高（编辑器行数 + 状态栏行 + 边框）。
    pub rows: u16,
    /// 编辑器本体（文本 + 光标 + undo）。组件归组件，状态归子区。
    pub editor: Editor,
    /// 输入历史（内存环，上限 20 条，不落盘）。
    pub history: History,
    /// 是否处于"空闲可回溯"状态：true = 输入框为空、允许 ↑↓ 回溯
    /// 历史消息；一旦开始打字立即置 false。hang 的反面参照。
    pub finished: bool,
    /// 补全挂钩：true = ↑↓（及 Tab）的解释权让给保留区所有者。
    /// 触发条件：光标正落在触动词**里面**（行首命令词、路径词）。
    pub hang: bool,
    /// 补全弹窗开着（由 Zone 每轮从补全服务同步进来）。
    ///
    /// 与 `hang` 是两件事：`hang` 说的是"这个词可以补全"（词内），
    /// 这里说的是"弹窗正开着、有候选等着被选中"（命令名打完之后也成立）。
    pub popup_open: bool,
    /// `/usage` 面板开着（由 Zone 每轮从保留区同步进来）。
    ///
    /// 面板是**只读的一屏数**：它只借 Esc，其余键照常进编辑器。但 Esc 必须
    /// 先归它——面板在上层，那一下的意思就是"关掉它"。
    pub panel_open: bool,
    /// 上一次↑↓的目标列记忆（Editor::up/down 的 goal_col 载体）。
    pub goal_col: Option<usize>,
    /// 待投递的借用请求：deliver 里产出，Zone 取走并转发保留区。
    /// 每轮事件最多一条；Zone 取走后清空。
    pub pending_lend: Option<ToReserved>,
    /// 待上报的会话出口请求。Zone 逐级取走（input → main → app → loop）。
    /// 出口请求**队列**，不是单槽。
    ///
    /// 一个事件批次里可能来两次 Esc（用户手快，或者两次按键被合并投递），
    /// 单槽会吞掉第一次——两下 Esc 开节点树就永远凑不齐。队列让每一次
    /// 都原样上报，什么时候处理是会话层的事。
    pub pending_exits: Vec<ExitRequest>,
    /// 会话是否在回复中（daemon 的 wire state）。Esc 在 streaming 时是
    /// 「打断」。只有 app 能写。
    pub streaming: bool,
    /// 上一次「留在本区的空闲 Esc」的时刻。两下之内再来一下 = 开节点树。
    ///
    /// **借出去的 Esc 不进来**：面板、补全弹窗、hang 都会先把 Esc 借走，
    /// 那几下是"把上面这层收掉"，不是"我要开树"。借走的同时本字段被清空，
    /// 免得半截的双击接上后面那一下。
    esc_armed: Option<std::time::Instant>,
    /// 光标的终端绝对列（含 `+- ` 边框 2 格铺垫）。补全弹窗锚定用；
    /// 每次渲染后重算。
    pub cursor_col: u16,
    /// 光标在**容器内**的行偏移（0 = 状态栏那一行）。硬件光标定位用：
    /// 子区报容器内的位置，换算成终端坐标是主区的事。
    pub cursor_row: u16,
    /// 状态栏（容器顶边）。它自己是一套自更新部件的注册表：外部
    /// 通过 [`InputZone::notify`] 投事件，它自己决定谁更新、怎么画。
    pub statusline: StatusLine,
    /// 本轮仲裁拿到的终端尺寸。键位在两次 resize 之间随时到达，
    /// 换行要用它，不能等下一次渲染。
    term: crate::tui::zone::TermSize,
}

impl Default for InputZone {
    fn default() -> Self {
        Self {
            scroll: 0,
            rows: 0,
            editor: Editor::new(),
            history: History::new(),
            finished: true,
            hang: false,
            popup_open: false,
            panel_open: false,
            goal_col: None,
            pending_lend: None,
            pending_exits: Vec::new(),
            streaming: false,
            esc_armed: None,
            cursor_col: geometry::BORDER_COLS as u16,
            cursor_row: 1,
            statusline: StatusLine::new(),
            term: crate::tui::zone::TermSize {
                cols: 80,
                rows: 24,
            },
        }
    }
}

impl InputZone {
    /// 自限：按内容算自己要多少行，上限 ≤ 终端 1/4（算术归 layout，
    /// 申请归子区）。
    pub fn request_height_for(&self, wrapped_lines: usize, term_h: u16) -> u16 {
        geometry::container_height(term_h, wrapped_lines)
            .try_into()
            .unwrap_or(u16::MAX)
    }

    /// 状态栏事件入口。喂事件不是 TUI 的活（会话层重构后接上），
    /// 这里只把通道留好：谁有事实谁通知，状态栏自己更新自己。
    pub fn notify(&mut self, ev: &StatusEvent<'_>) {
        self.statusline.notify(ev);
    }

    /// 本子区当前宽度下的换行结果（键位处理与渲染共用同一份算术）。
    fn wrapped(&self) -> crate::tui::text::Wrapped {
        crate::tui::text::wrap(&self.editor.text(), geometry::inner_width(self.term.cols))
    }
}

impl InputZone {
    /// 动作处理：编辑动作驱动 Editor，历史动作驱动 History 环。
    /// 返回true = 动作已消费。
    fn apply(&mut self, action: &semantics::Action, wrapped: Option<&crate::tui::text::Wrapped>) -> bool {
        use semantics::Action as A;
        // Submit/Clear 自己管理 finished；走此标记时跳过 epilogue 的统一翻动。
        let mut self_managed = false;
        let eff = match action {
            A::Insert(c) => self.editor.insert_char(*c),
            A::Paste(s) => {
                let before = self.editor.len();
                let _ = self.editor.insert_paste(s);
                // insert_paste 的 bool 只表示"折叠成标记"，小段粘贴
                // 直接落字也算 Content（内容确实变了）。
                if self.editor.len() != before {
                    Effect::Content
                } else {
                    Effect::Nothing
                }
            }
            A::Newline => self.editor.insert_str("\n"),
            A::Backspace => self.editor.backspace(),
            A::Delete => self.editor.delete(),
            A::Left => self.editor.left(),
            A::Right => self.editor.right(),
            A::Up if !self.editor_is_multiline() || self.history.browsing() => {
                // 单行/已回溯：↑ 走历史。
                if self.finished || self.history.browsing() {
                    if let Some(text) = self.history.previous(&self.editor.text()) {
                        self.editor = Editor::from_text(&text);
                    }
                    return true;
                }
                Effect::Nothing
            }
            A::Up => self.editor.up(wrapped_expect(wrapped), &mut self.goal_col),
            A::Down if self.history.browsing() => {
                if let Some(text) = self.history.next_entry() {
                    self.editor = Editor::from_text(&text);
                }
                return true;
            }
            A::Down => self.editor.down(wrapped_expect(wrapped), &mut self.goal_col),
            A::LineHome => self.editor.line_home(),
            A::LineEnd => self.editor.line_end(),
            A::DocHome => self.editor.home(),
            A::DocEnd => self.editor.end(),
            A::WordLeft => self.editor.word_left(),
            A::WordRight => self.editor.word_right(),
            A::DeleteWordBackward => self.editor.delete_word_backward(),
            A::DeleteWordForward => self.editor.delete_word_forward(),
            A::DeleteToLineStart => self.editor.delete_to_line_start(),
            A::DeleteToLineEnd => self.editor.delete_to_line_end(),
            A::Undo => {
                self.editor.undo();
                Effect::Content
            }
            A::Redo => {
                self.editor.redo();
                Effect::Content
            }
            // 出口三动作：编辑器只声明"要出去"，去向归会话层。
            A::Submit => {
                // What goes out is the **expanded** text. The editor holds
                // folded markers; their originals live only in `PasteStore`,
                // which `clear()` below drops — so reading `text()` here would
                // send `[paste #1 +23 lines]` to the model and lose the paste.
                let text = self.editor.expanded_text();
                if text.trim().is_empty() {
                    Effect::Nothing // 空输入不提交
                } else {
                    // History keeps the folded form: ↑ should recall one line,
                    // not thirty.
                    self.history.push(self.editor.text());
                    self.pending_exits.push(ExitRequest::Submit(text));
                    self.editor.clear();
                    self.finished = true;
                    self_managed = true;
                    Effect::Content
                }
            }
            A::Quit => {
                self.pending_exits.push(ExitRequest::Quit);
                Effect::Nothing
            }
            A::Interrupt => {
                self.pending_exits.push(ExitRequest::Interrupt);
                Effect::Nothing
            }
            A::ClearInput => {
                self.editor.clear();
                self.history.exit();
                self.finished = true;
                self_managed = true;
                Effect::Content
            }
            // 以下动作的解释权不在编辑器内（折叠归历史区，补全/hang
            // 归保留区联动——接线后逐条归位）。
            A::HistoryPrev | A::HistoryNext => Effect::Nothing,
            A::Complete | A::CompleteUp | A::CompleteDown | A::DismissCompletion => {
                Effect::Nothing
            }
            // 面板的 Esc：不是编辑动作，解释权在保留区（借出去，见 `deliver`）。
            A::DismissPanel => Effect::Nothing,
            A::ToggleReasoning | A::ToggleTools => Effect::Nothing,
            A::None => Effect::Nothing,
        };
        match eff {
            Effect::Nothing => false,
            _ => {
                // 内容一动就退出 finished（started typing）。
                // Submit/ClearInput 自己管理 finished（提交后是 true），
                // 跳过统一翻动。
                if matches!(eff, Effect::Content) {
                    if !self_managed {
                        self.finished = false;
                    }
                    // 编辑历史回捞的文本 = 它变成新输入，退出回溯
                    // （草稿语义见 editor::history::on_edit）。
                    self.history.on_edit();
                }
                true
            }
        }
    }

    /// 光标是否在首行（历史回溯的前置条件之一）。
    fn editor_is_multiline(&self) -> bool {
        self.editor.text().contains('\n')
    }

    /// 回退收回了那句话：原样放进输入框，光标在末尾。编辑器里已有的草稿
    /// **不覆盖**——收回的是"这句话"，不是"清空你正在写的东西"；两者都
    /// 在的话，新文本接在草稿后面，用户自己删。
    pub fn prefill(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if self.editor.is_empty() {
            self.editor = Editor::from_text(text);
            self.finished = false;
            self.history.on_edit();
        }
    }

    /// 同步补全弹窗的开合（Zone 每轮调用）。弹窗开着时 ↑↓/Tab/Enter/Esc
    /// 归补全服务——见 `deliver` 里那条分支。
    pub fn set_completion_open(&mut self, open: bool) {
        self.popup_open = open;
    }

    /// 同步 `/usage` 面板的开合（Zone 每轮调用）。开着时 Esc 归面板。
    pub fn set_panel_open(&mut self, open: bool) {
        self.panel_open = open;
    }

    /// **Esc 的归属判定，全程序唯一一处。**
    ///
    /// 落在本区的 Esc 有五种去向，按上到下的层级排：
    ///
    /// 1. `/usage` 面板开着 → 借出去关面板。
    /// 2. 补全弹窗开着 → 借出去关弹窗。
    /// 3. hang（光标在触发词里）→ 借出去关弹窗（同一条路，弹窗可能已经
    ///    关了，那就没人接，丢掉）。理由：人正写着半截路径，那一下的
    ///    意思不可能是"打断远处那条回复"。
    /// 4. 在回复中 → **打断**。
    /// 5. 其余（空闲）→ 数窗口：`ESC_TREE_WINDOW` 内第二下 = 开节点树。
    ///
    /// 前三类**借走即清窗口**，且不产生任何出口请求。第四类同理。只有
    /// 第五类才进窗口计数——所以"两下 Esc 开树"在面板/弹窗/hang 之下
    /// 天然不成立，不需要在会话层再判一次状态。
    ///
    /// 返回 true = 已消费。带 Alt/Ctrl 的 Esc 不归这里（组合键另有归宿）。
    fn route_esc(&mut self, key: &ratatui::crossterm::event::KeyEvent) -> bool {
        use ratatui::crossterm::event::KeyModifiers;
        if key
            .modifiers
            .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL)
        {
            return false;
        }
        // **现场重算**，不用缓存：`hang` 只在编辑动作之后刷新，而 Esc 走的是
        // 提前返回那条路——它会一直看到上一次编辑留下的值。真机症状就是
        // "resume 之后空了输入框，两下 Esc 没反应，先敲一个空格再删掉又好了"：
        // 空格和删除都会重算 `hang`，Esc 不会。
        self.refresh_hang();
        use semantics::Action as A;
        let borrowed = if self.panel_open {
            Some(A::DismissPanel)
        } else if self.popup_open || self.hang {
            Some(A::DismissCompletion)
        } else {
            None
        };
        crate::tui::session::keylog(&format!(
            "  route_esc panel={} popup={} hang={} streaming={} borrowed={} armed={}",
            self.panel_open,
            self.popup_open,
            self.hang,
            self.streaming,
            borrowed.is_some(),
            self.esc_armed.is_some(),
        ));
        if let Some(action) = borrowed {
            // 半截的双击不许接上后面那一下：借走就作废。
            self.esc_armed = None;
            self.pending_lend = Some(ToReserved::Lend(action));
            return true;
        }
        // 回合在跑：打断。Esc 的打断语义是固定的，别处没有开关。
        if self.streaming {
            self.esc_armed = None;
            self.pending_exits.push(ExitRequest::Interrupt);
            return true;
        }
        let now = std::time::Instant::now();
        match self.esc_armed.take() {
            Some(t) if now.duration_since(t) < ESC_TREE_WINDOW => {
                crate::tui::session::keylog(&format!(
                    "    -> OpenTree (gap {}ms)",
                    now.duration_since(t).as_millis()
                ));
                self.pending_exits.push(ExitRequest::OpenTree);
            }
            _ => self.esc_armed = Some(now),
        }
        true
    }

    /// 补全确认回执：把保留区服务算出的替换写回编辑器，随后自然
    /// 走 refresh_hang 重算挂起状态。字符索引与引擎一致（char index）。
    /// 提交当前输入（与 `Action::Submit` 同一条路）。
    ///
    /// 给 Zone 用：hang 时 Enter 先被当成"确认候选"投给保留区，保留区没有
    /// 候选可确认时，这一下 Enter 必须落回提交——否则 `/compact` 这种
    /// **无参数的叶子命令**会白吃一次回车（精确命中 → 补全弹窗关闭 →
    /// 没有可确认的东西 → 输入永远发不出去）。
    pub fn submit_now(&mut self) -> bool {
        self.apply(&semantics::Action::Submit, None)
    }

    pub fn apply_completion(&mut self, action: crate::tui::zone::main::reserved::completion::engine::CompletionAction) {
        use crate::tui::zone::main::reserved::completion::engine::CompletionAction;
        if let CompletionAction::Replace { from, to, text } = action {
            self.editor.replace_range(from, to, &text);
            self.finished = false;
        }
    }

    /// hang 触发点：从编辑器当前光标处回扫触发词。
    ///
    /// 词的边界 = SeparatorTable 硬边界（`./foo src/main.rs` 各是一词）。
    /// 触发形态（用户决策，两种）：
    ///
    /// - 行首命令：全文第一个字符是 `/` 且光标在第一词内；
    /// - 路径：词含 `/` 且前方硬边界是空格，或词以 `./` `../` `~` 开头。
    ///
    /// 解出 → `hang = true`；解不出 → `hang = false`
    /// （完成通知撤 hang 就是这条路径：确认/删除触发词后自然回落）。
    fn refresh_hang(&mut self) {
        let text = self.editor.text();
        let cursor = self.editor.cursor();
        let chars: Vec<char> = text.chars().collect();
        let cursor = cursor.min(chars.len());
        // 回扫到最近硬边界。
        let mut start = cursor;
        while start > 0 && !self.editor.separators.is_hard(chars[start - 1]) {
            start -= 1;
        }
        let word: String = chars[start..cursor].iter().collect();
        // 前置必须是空格或文本开头（用户决策：仅空格触发）。
        let spaced = start == 0 || chars[start - 1] == ' ';
        // 触发形态：
        // - 行首命令：全文首字符 `/`（光标在命令词内）；
        // - 路径词：显式前缀 ./ ../ ~；或裸相对路径——词首段（`/` 之前）
        //   是 ASCII 路径段（字母数字-_），`看这个/吧` 这种中文正文里的
        //   斜杠不是路径。
        let is_command = start == 0 && word.starts_with('/');
        let head = word.split('/').next().unwrap_or("");
        let ascii_head = !head.is_empty()
            && head
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
        let is_path = word.starts_with("./")
            || word.starts_with("../")
            || word.starts_with("~/")
            || word.starts_with('/')
            || (word.contains('/') && ascii_head);
        self.hang = spaced && !word.is_empty() && (is_command || is_path);
    }
}

/// hang 状态下**翻译之后**还能让给保留区的动作：只有 ↑↓。
///
/// Tab / Enter（确认候选）与 Esc（收起补全）不在这里——它们在 `deliver`
/// 的挂起分支里就被截下来了，根本走不到翻译表。留在这里的这两个是"翻译成
/// 了编辑器动作、但解释权其实在补全服务"的：↑↓ 在补全服务手里是换高亮。
fn hang_lendable(action: &semantics::Action) -> Option<semantics::Action> {
    use semantics::Action as A;
    match action {
        A::Up | A::Down => Some(action.clone()),
        _ => None,
    }
}

/// Editor::up/down 需要 Wrapped。调用方按当前终端宽度换好传进来；
/// 拿不到时用单行空结果兜底（空输入的上下本来就走历史回溯分支）。
fn empty_wrapped() -> &'static crate::tui::text::Wrapped {
    static EMPTY: std::sync::LazyLock<crate::tui::text::Wrapped> =
        std::sync::LazyLock::new(|| crate::tui::text::wrap("", 1));
    &EMPTY
}

fn wrapped_expect(w: Option<&crate::tui::text::Wrapped>) -> &crate::tui::text::Wrapped {
    w.unwrap_or_else(|| empty_wrapped())
}

impl SubZone for InputZone {
    /// 自限：按内容换行后的行数算容器高（上限 1/4 终端高）。
    fn request_height(&self, term: crate::tui::zone::TermSize) -> u16 {
        self.request_height_for(self.wrapped_for(term).len(), term.rows)
    }

    fn assign(&mut self, term: crate::tui::zone::TermSize, rows: u16) {
        self.term = term;
        self.rows = rows;
    }

    fn on_height_changed(&mut self, _rows: u16) -> bool {
        false // 自限行高，不订阅广播
    }

    /// 声明：键盘 + 粘贴。滚轮不声明——回滚是历史区的语义。
    fn accepts(&self) -> &'static [crate::tui::zone::RawEventKind] {
        use crate::tui::zone::RawEventKind;
        &[RawEventKind::Key, RawEventKind::Paste]
    }

    /// 分发入口：编辑键的最终消费者是本子区。顺序：
    /// hang 成立 → 翻译后把可让出的动作打包 `ToReserved::Lend` 投给
    /// 保留区（Zone 代传达，收件人写死）；保留区不收的动作退回，
    /// 本子区自己消化。hang 触发点未实现，现恒 false。
    fn deliver(&mut self, event: &crate::tui::zone::RawEvent) -> bool {
        // **Esc 先过归属判定**：这一步在所有人之前，因为借走它的那几家
        // （面板 / 补全 / hang）都只处理"别的键"，Esc 落到它们手里会被
        // 当成没认出来。归属在这里一次定死——见 `route_esc`。
        if let crate::tui::zone::RawEvent::Key { key } = event
            && key.code == KeyCode::Esc
            && self.route_esc(key)
        {
            return true;
        }
        // 补全弹窗**开着**：↑↓/Tab/Enter/Esc 全归补全服务。确认（Tab/Enter）
        // 走 Confirm——服务把选中项算成替换文本，Zone 带回编辑器写回；其余
        // （↑↓ 换高亮、Esc 关弹窗）原样借出去。
        //
        // 以前只有一个 `hang`（光标在触发词**里**）会借：`/switch ` 这种
        // "命令名打完了、正在填参数"的状态下 hang 已经落到 false，于是弹窗
        // 虽然开着，回车却落回提交，↑↓ 也动不了高亮。
        // 面板开着：Esc 关、上下选模型、Tab 换口径——**全归它**。先于补全判断，
        // 也先于编辑器：面板是浮层，它在上层，输入框里那几行字等关了再编辑不迟。
        if self.panel_open
            && let RawEvent::Key { key } = event
        {
            use semantics::Action as A;
            let cx = semantics::KeyContext {
                panel_open: true,
                ..Default::default()
            };
            let action = semantics::translate_with(*key, cx);
            // Esc 不在这里——它在 `route_esc` 里就归了面板。
            if matches!(action, A::Up | A::Down | A::Complete) {
                self.pending_lend = Some(ToReserved::Lend(action));
                return true;
            }
        }
        if self.popup_open
            && let RawEvent::Key { key } = event
        {
            let cx = semantics::KeyContext::for_input(false, self.streaming, true);
            let action = semantics::translate_with(*key, cx);
            use semantics::Action as A;
            // 空格在这里也是"确认"：路径参数打完（`/cdp ~/Projects/Utility/MyPi `）
            // 那一下空格，人想说的是"就到这儿"。落进编辑器它就变成参数里的
            // 一个空格，命令解析跟着错位。
            let space = key.code == KeyCode::Char(' ')
                && !key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL);
            match action {
                A::Complete => {
                    self.pending_lend = Some(ToReserved::Confirm);
                    return true;
                }
                A::CompleteUp | A::CompleteDown => {
                    self.pending_lend = Some(ToReserved::Lend(action));
                    return true;
                }
                _ if space => {
                    self.pending_lend = Some(ToReserved::Confirm);
                    return true;
                }
                _ => {}
            }
        }
        // hang 时输入区自己决定确认键：Tab 和 Enter 都是"确认候选"
        // （解释权在最终消费者——这里——不写死在任何翻译表里）。
        if self.hang
            && let RawEvent::Key { key } = event
        {
            let plain_enter = key.code == KeyCode::Enter
                && !key
                    .modifiers
                    .intersects(KeyModifiers::ALT | KeyModifiers::CONTROL);
            let tab = key.code == KeyCode::Tab;
            if plain_enter || tab {
                self.pending_lend = Some(ToReserved::Confirm);
                return true;
            }
        }
        let action = match event {
            RawEvent::Key { key } => {
                let mut cx = semantics::KeyContext::for_input(
                    self.editor.is_empty(),
                    self.streaming,
                    self.popup_open,
                );
                cx.panel_open = self.panel_open;
                semantics::translate_with(*key, cx)
            }
            RawEvent::Paste(s) => semantics::Action::Paste(s.clone()),
            // 滚轮未声明，兜底防御。
            RawEvent::ScrollUp | RawEvent::ScrollDown => return false,
        };
        if matches!(action, semantics::Action::None) {
            return false;
        }
        // hang：↑↓（及 Tab）的解释权让给保留区所有者。借出动作由
        // 保留区所有者的 Claim::accepts() 自取，退回的本区消化。
        if self.hang
            && let Some(lendable) = hang_lendable(&action)
        {
            self.pending_lend = Some(ToReserved::Lend(lendable));
            return true;
        }
        // 只有上下移动需要换行结果（跨行跳要按视觉行命中）；别的动作
        // 不换，省一次 wrap。
        let wrapped = matches!(action, semantics::Action::Up | semantics::Action::Down)
            .then(|| self.wrapped());
        let consumed = self.apply(&action, wrapped.as_ref());
        // hang 触发：每次编辑后从编辑器解出"触发词"。定义全部来自
        // SeparatorTable（硬边界分词）；解出 path/command 形态 → 置
        // hang（↑↓Tab 解释权让给保留区服务）；解不出 → 撤 hang。
        self.refresh_hang();
        consumed
    }
}

impl InputZone {
    /// 本子区在给定终端宽度下的换行结果（request_height 用的是
    /// 还没存下来的新尺寸，所以单独收一个宽度）。
    fn wrapped_for(&self, term: crate::tui::zone::TermSize) -> crate::tui::text::Wrapped {
        crate::tui::text::wrap(&self.editor.text(), geometry::inner_width(term.cols))
    }

    /// 自渲染。签名与兄弟子区一致（宽度传参，不存字段）。
    ///
    /// 顺序：换行 → 光标跟随滚动 → 状态栏那一行 → 拼容器。
    /// 光标绝对列顺手报给补全弹窗（锚点）用。
    pub fn render_rows(
        &mut self,
        term_w: u16,
        p: &crate::tui::theme::Palette,
    ) -> Vec<ratatui::text::Line<'static>> {
        let term_w = term_w.max(1);
        let wrapped = crate::tui::text::wrap(&self.editor.text(), geometry::inner_width(term_w));
        let (cursor_row, cursor_col) = wrapped.locate(self.editor.cursor());
        let container_rows = usize::from(self.rows.max(2));
        self.scroll = geometry::adjust_scroll(self.scroll, wrapped.len(), container_rows, cursor_row);
        let status_line = self.statusline.render(&StatusTheme::resolve(), term_w);
        let view = render::render(
            &render::Spec {
                status_line,
                wrapped: &wrapped,
                starts: self.scroll,
                container_rows,
                cursor_row,
                cursor_col,
                term_w,
            },
            p,
        );
        self.cursor_col = view.cursor_col as u16;
        self.cursor_row = view.cursor_row as u16;
        view.lines
    }
}

pub mod semantics;

pub use semantics::{translate, translate_with, Action, KeyContext};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::zone::RawEventKind;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(c: char) -> RawEvent {
        RawEvent::Key {
            key: KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
        }
    }

    #[test]
    fn typing_consumes_and_flips_finished() {
        let mut z = InputZone::default();
        assert!(z.finished);
        assert!(z.deliver(&key('h')));
        assert!(z.deliver(&key('i')));
        assert_eq!(z.editor.text(), "hi");
        assert!(!z.finished, "开始打字后 finished 必须翻 false");
    }

    #[test]
    fn ctrl_arrows_word_jump() {
        let mut z = InputZone::default();
        for c in "hello world".chars() {
            z.deliver(&key(c));
        }
        z.deliver(&RawEvent::Key {
            key: KeyEvent {
                code: KeyCode::Left,
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
        });
        assert_eq!(z.editor.cursor(), 6);
    }

    #[test]
    fn backspace_consumes_but_empty_editor_still_reports() {
        let mut z = InputZone::default();
        z.deliver(&key('a'));
        z.deliver(&RawEvent::Key {
            key: KeyEvent {
                code: KeyCode::Backspace,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
        });
        assert!(z.editor.is_empty());
    }

    #[test]
    fn paste_enters_editor_and_unfinishes() {
        let mut z = InputZone::default();
        assert!(z.deliver(&RawEvent::Paste("ab\ncd".into())));
        assert_eq!(z.editor.text(), "ab\ncd");
        assert!(!z.finished);
    }

    #[test]
    fn hang_lends_up_down_tab_sets_pending() {
        let mut z = InputZone {
            hang: true,
            ..InputZone::default()
        };
        z.deliver(&RawEvent::Key {
            key: KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
        });
        assert!(z.pending_lend.is_some(), "hang 时 ↑ 必须产生借用包裹");
        // 非 hang 状态同键走编辑器（光标移动），不产生包裹。
        let mut z2 = InputZone::default();
        assert!(z2.deliver(&RawEvent::Key {
            key: KeyEvent {
                code: KeyCode::Up,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
        }));
        assert!(z2.pending_lend.is_none());
    }

    /// **挂起时 Esc 不许打断。**
    ///
    /// 真机 bug：正在写半截路径（补全挂着），按 Esc 想收掉补全，结果远处那条
    /// 回复被打断了。挂起意味着"用户此刻在输入框里干活"，那一下 Esc 的解释权
    /// 不在会话层的"打断"上。
    #[test]
    fn esc_while_hung_lends_instead_of_interrupting() {
        let esc = || RawEvent::Key {
            key: KeyEvent {
                code: KeyCode::Esc,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
        };
        // 用**真的触发词**把 hang 立起来，而不是伪造字段——`route_esc` 现在
        // 现场重算 hang，伪造的 `hang: true` 配空编辑器会被一眼看穿。
        let mut z = InputZone {
            streaming: true,
            ..InputZone::default()
        };
        for c in "/mode".chars() {
            z.deliver(&key(c));
        }
        assert!(z.hang, "前提：行首斜杠词必须挂起");
        assert!(z.deliver(&esc()));
        assert!(
            z.pending_exits.is_empty(),
            "挂起时 Esc 不该打断：{:?}",
            z.pending_exits
        );
        assert!(
            matches!(z.pending_lend, Some(ToReserved::Lend(_))),
            "Esc 该被借给保留区（没人接就丢掉），而不是落回翻译表"
        );

        // 没挂起：Esc 照旧打断正在跑的回复。
        let mut z2 = InputZone {
            streaming: true,
            ..InputZone::default()
        };
        z2.deliver(&esc());
        assert_eq!(z2.pending_exits, vec![ExitRequest::Interrupt]);

        // 挂起且空闲：借出去关补全，不产生任何出口请求（不该去数窗口）。
        let mut z3 = InputZone::default();
        for c in "/mode".chars() {
            z3.deliver(&key(c));
        }
        assert!(z3.hang, "前提：挂起");
        z3.deliver(&esc());
        assert!(z3.pending_exits.is_empty(), "挂起时 Esc 只借不报");
    }

    #[test]
    fn accepts_declares_key_and_paste_only() {
        let z = InputZone::default();
        assert_eq!(
            z.accepts(),
            &[RawEventKind::Key, RawEventKind::Paste]
        );
    }
}

#[cfg(test)]
mod exit_tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(code: KeyCode, mods: KeyModifiers) -> RawEvent {
        RawEvent::Key {
            key: KeyEvent {
                code,
                modifiers: mods,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
        }
    }

    fn type_str(z: &mut InputZone, s: &str) {
        for c in s.chars() {
            z.deliver(&key(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    #[test]
    fn enter_submits_pushes_history_and_clears() {
        let mut z = InputZone::default();
        type_str(&mut z, "hi");
        z.deliver(&key(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(
            z.pending_exits,
            vec![ExitRequest::Submit("hi".into())]
        );
        assert!(z.editor.is_empty(), "提交后编辑器必须清空");
        assert!(z.finished);
        assert_eq!(z.history.len(), 1);
    }

    /// **大段粘贴必须展开后才发出去。**
    ///
    /// 这是真机上的 bug：编辑器里存的是折叠标记 `[paste #1 +30 lines]`，原文
    /// 只在 `PasteStore` 里，而提交会 `clear()` 把它一起丢掉。提交路径读了
    /// `text()`（未展开），于是模型收到的只有一个标记，粘贴的内容整个消失。
    #[test]
    fn submit_sends_the_expanded_paste_not_the_marker() {
        let mut z = InputZone::default();
        let body: String = (0..30).map(|i| format!("code line {i}\n")).collect();
        z.editor.insert_paste(&body);
        assert!(z.editor.text().contains("[paste #1"), "这么多行该折叠成标记");

        z.deliver(&key(KeyCode::Enter, KeyModifiers::NONE));

        let sent = match &z.pending_exits[0] {
            ExitRequest::Submit(t) => t.clone(),
            other => panic!("应该是 Submit，实际 {other:?}"),
        };
        assert!(sent.contains("code line 29"), "原文必须在：{sent}");
        assert!(!sent.contains("[paste #"), "标记本身不该发出去：{sent}");
        assert!(sent.contains("```pasted content"), "要带围栏：{sent}");
        assert!(sent.trim_end().ends_with("```"), "围栏要闭合：{sent}");
    }

    /// 空闲时两下 Esc = 开节点树。
    ///
    /// 真机上这里是"按两下没反应"。两个原因：出口请求原来是单槽（第二次被
    /// 静默吞掉），以及窗口数在会话层（它看不见面板/补全/hang 把 Esc 借走
    /// 这件事）。现在窗口在输入区里，两下连着到就是一条 `OpenTree`。
    #[test]
    fn two_escapes_open_the_tree() {
        let mut z = InputZone::default();
        z.deliver(&key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(z.pending_exits.is_empty(), "第一下只是记时刻，不产生请求");
        z.deliver(&key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(
            z.pending_exits,
            vec![ExitRequest::OpenTree],
            "两下连着到 = 开树"
        );
    }

    /// **被借走的 Esc 不参与双击窗口。**
    ///
    /// 面板 / 补全弹窗 / hang 都会先拿走 Esc，那几下的意思是"把上面这层
    /// 收掉"，不是"我要看节点树"。它们既不能开树，也不能把窗口里的半截
    /// 双击接上——否则"关弹窗 + 随手再按一下"会莫名其妙跳出全屏树。
    #[test]
    fn borrowed_escapes_do_not_open_the_tree() {
        // 挂起时连按三下：全部借走，一条出口请求都没有。
        let mut z = InputZone::default();
        type_str(&mut z, "/mode");
        assert!(z.hang, "前提：挂起");
        for _ in 0..3 {
            z.deliver(&key(KeyCode::Esc, KeyModifiers::NONE));
        }
        assert!(z.pending_exits.is_empty(), "{:?}", z.pending_exits);

        // 窗口里已经有一下，再来的一下被借走 → 窗口作废，
        // 借走之后的那一下必须重新起算。
        let mut z = InputZone::default();
        z.deliver(&key(KeyCode::Esc, KeyModifiers::NONE));
        z.popup_open = true;
        z.deliver(&key(KeyCode::Esc, KeyModifiers::NONE));
        z.popup_open = false;
        z.deliver(&key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(
            z.pending_exits.is_empty(),
            "被借走作废窗口后，剩下的那一下只是新的第一下：{:?}",
            z.pending_exits
        );
    }

    /// **陈旧的 `hang` 不许拦住两下 Esc。**
    ///
    /// 真机症状：resume 之后输入框是空的，两下 Esc 不开树；先敲一个空格、
    /// 再删掉，就好了。根因是 `hang` 只在**编辑动作之后**重算，而 Esc 走
    /// `route_esc` 提前返回那条路——它会一直看到上一次编辑留下的值。空格
    /// 和删除都会重算，Esc 不会，所以看起来像"敲一下才好"。
    #[test]
    fn a_stale_hang_does_not_block_the_tree() {
        let mut z = InputZone {
            hang: true, // 陈旧值：输入框其实已经空了
            ..InputZone::default()
        };
        assert!(z.editor.is_empty(), "前提：输入框是空的");
        z.deliver(&key(KeyCode::Esc, KeyModifiers::NONE));
        z.deliver(&key(KeyCode::Esc, KeyModifiers::NONE));
        assert_eq!(
            z.pending_exits,
            vec![ExitRequest::OpenTree],
            "空的输入框 + 陈旧的 hang，两下 Esc 照样开树"
        );
    }

    /// **`Ctrl+C` 在回合进行中是打断**，不是清空。
    #[test]
    fn ctrl_c_interrupts_while_a_reply_is_running() {
        let mut z = InputZone {
            streaming: true,
            ..InputZone::default()
        };
        for c in "草稿".chars() {
            z.deliver(&key(KeyCode::Char(c), KeyModifiers::NONE));
        }
        z.deliver(&key(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(z.pending_exits, vec![ExitRequest::Interrupt]);
        assert_eq!(z.editor.text(), "草稿", "打断不该顺手清掉草稿");
    }

    #[test]
    fn empty_enter_does_not_submit() {
        let mut z = InputZone::default();
        z.deliver(&key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(z.pending_exits.is_empty());
        // 纯空白同样不提交
        type_str(&mut z, "   ");
        z.deliver(&key(KeyCode::Enter, KeyModifiers::NONE));
        assert!(z.pending_exits.is_empty());
    }

    #[test]
    fn idle_esc_does_not_quit() {
        // Esc 的语义只有"打断"和"连按两下开树"。空闲单按**什么请求都不
        // 产生**——尤其不能退出程序（那是 Ctrl+C / Ctrl+D 的事）。
        let mut z = InputZone::default();
        z.deliver(&key(KeyCode::Esc, KeyModifiers::NONE));
        assert!(
            z.pending_exits.is_empty(),
            "空闲单按 Esc 只是记时刻：{:?}",
            z.pending_exits
        );
    }

    #[test]
    fn ctrl_c_clears_empty_input_and_quits_only_via_finished_flag() {
        // Ctrl+C 语义 = 清空输入（semantics::translate 对空编辑器给 Quit）。
        let mut z = InputZone::default();
        type_str(&mut z, "abc");
        z.deliver(&key(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(z.editor.is_empty());
        assert!(z.finished);
        assert!(z.pending_exits.is_empty());
    }

    #[test]
    fn history_up_recalls_then_down_returns_draft() {
        let mut z = InputZone::default();
        // 先造一条历史：输入 old、提交。
        type_str(&mut z, "old");
        z.deliver(&key(KeyCode::Enter, KeyModifiers::NONE));
        // finished 空框：↑ 回溯。
        z.deliver(&key(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(z.editor.text(), "old");
        // 打一个字 → 退出回溯。
        type_str(&mut z, "x");
        assert!(!z.history.browsing());
    }
}

#[cfg(test)]
mod hang_tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};

    fn key(c: char) -> RawEvent {
        RawEvent::Key {
            key: KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
        }
    }

    fn type_str(z: &mut InputZone, s: &str) {
        for c in s.chars() {
            z.deliver(&key(c));
        }
    }

    #[test]
    fn slash_at_start_sets_hang() {
        let mut z = InputZone::default();
        type_str(&mut z, "/mode");
        assert!(z.hang, "行首斜杠命令词必须挂起");
    }

    #[test]
    fn slash_after_text_does_not_hang() {
        let mut z = InputZone::default();
        type_str(&mut z, "看这个/吧");
        assert!(!z.hang, "非行首的斜杠不是命令触发");
    }

    #[test]
    fn space_then_relative_path_sets_hang() {
        let mut z = InputZone::default();
        type_str(&mut z, "cd ./to");
        assert!(z.hang, "空格后的 ./ 路径必须挂起");
    }

    #[test]
    fn bare_relative_path_sets_hang() {
        let mut z = InputZone::default();
        type_str(&mut z, "src/ma");
        assert!(z.hang, "空格后的裸相对路径必须挂起（文本开头也算）");
    }

    #[test]
    fn plain_word_no_hang() {
        let mut z = InputZone::default();
        type_str(&mut z, "hello");
        assert!(!z.hang);
    }

    #[test]
    fn block_deletion_keeps_path_whole() {
        let mut z = InputZone::default();
        type_str(&mut z, "src/main.rs end");
        // 光标在 "end" 末尾。退三词：end <- src/main.rs（整块） <- 空。
        z.deliver(&RawEvent::Key {
            key: KeyEvent {
                code: KeyCode::Backspace,
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
        });
        assert_eq!(z.editor.text(), "src/main.rs ", "只删掉 end");
        z.deliver(&RawEvent::Key {
            key: KeyEvent {
                code: KeyCode::Backspace,
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
        });
        assert_eq!(z.editor.text(), "", "src/main.rs 必须一删整块（连接符不切块）");
    }

    #[test]
    fn ctrl_left_jumps_whole_path() {
        let mut z = InputZone::default();
        type_str(&mut z, "src/main.rs end");
        z.deliver(&RawEvent::Key {
            key: KeyEvent {
                code: KeyCode::Left,
                modifiers: KeyModifiers::CONTROL,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
        });
        // 一滑到 "src/main.rs " 之后（跳过 end 和空格）。
        assert_eq!(z.editor.cursor(), "src/main.rs ".chars().count());
    }
}

#[cfg(test)]
mod confirm_roundtrip {
    use super::*;
    use crate::tui::zone::{main::MainZone, Zone as _};

    fn type_str(mz: &mut MainZone, s: &str) {
        for c in s.chars() {
            mz.deliver(RawEvent::Key {
                key: ratatui::crossterm::event::KeyEvent {
                    code: KeyCode::Char(c),
                    modifiers: KeyModifiers::NONE,
                    kind: ratatui::crossterm::event::KeyEventKind::Press,
                    state: ratatui::crossterm::event::KeyEventState::NONE,
                },
            })
            ;
        }
    }

    fn tab(mz: &mut MainZone) {
        mz.deliver(RawEvent::Key {
            key: ratatui::crossterm::event::KeyEvent {
                code: KeyCode::Tab,
                modifiers: KeyModifiers::NONE,
                kind: ratatui::crossterm::event::KeyEventKind::Press,
                state: ratatui::crossterm::event::KeyEventState::NONE,
            },
        });
    }

    fn down(mz: &mut MainZone) {
        mz.deliver(RawEvent::Key {
            key: ratatui::crossterm::event::KeyEvent {
                code: KeyCode::Down,
                modifiers: KeyModifiers::NONE,
                kind: ratatui::crossterm::event::KeyEventKind::Press,
                state: ratatui::crossterm::event::KeyEventState::NONE,
            },
        });
    }

    fn enter(mz: &mut MainZone) {
        mz.deliver(RawEvent::Key {
            key: ratatui::crossterm::event::KeyEvent {
                code: KeyCode::Enter,
                modifiers: KeyModifiers::NONE,
                kind: ratatui::crossterm::event::KeyEventKind::Press,
                state: ratatui::crossterm::event::KeyEventState::NONE,
            },
        });
    }

    /// 无参数的**叶子命令**：回车必须一次提交。
    ///
    /// 行首斜杠词会挂起（`hang`，把补全的解释权让给保留区），于是回车先被
    /// 当成"确认候选"。`/compact` 这种精确命中会让补全弹窗**关闭**（leaf
    /// state），此时没有候选可确认——那一下回车必须落回提交，否则命令永远
    /// 发不出去（一个回车被白吃）。
    /// 有候选时，回车**确认候选**（不是提交）。
    ///
    /// 一条命令的参数有合法值（`/switch` 的模型 id），用户敲 Tab 看到列表、
    /// ↑↓ 选一条、回车——这一次回车必须把选中项写回编辑器，而不是把半截
    /// `/switch ` 发出去。发出去的下场是命令当场报"用法"，用户莫名其妙。
    #[test]
    fn enter_while_the_popup_is_open_confirms_the_candidate() {
        let tmp = std::env::temp_dir().join(format!("mypi_confirm_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut mz = MainZone::default();
        mz.attach(crate::tui::zone::TermSize { cols: 80, rows: 24 });
        mz.reserved.attach_completion(tmp.clone(), tmp.clone());
        mz.reserved.set_commands(vec![crate::server::wire::CommandInfo {
            name: "/switch".into(),
            aliases: vec![],
            detail: "换模型".into(),
            args: "model_id".into(),
            scope: "session".into(),
        }]);
        mz.reserved.set_candidates(
            vec![
                crate::tui::zone::main::reserved::completion::controller::ModelCandidate {
                    provider: "fake".into(),
                    id: "model-a".into(),
                    detail: "FAKE-A".into(),
                },
                crate::tui::zone::main::reserved::completion::controller::ModelCandidate {
                    provider: "fake".into(),
                    id: "model-b".into(),
                    detail: "FAKE-B".into(),
                },
            ],
            Vec::new(),
        );

        type_str(&mut mz, "/switch ");
        tab(&mut mz);
        assert!(
            mz.reserved.completion.as_ref().unwrap().controller.is_open(),
            "两个候选该把弹窗打开"
        );
        // 选第二条，回车。
        down(&mut mz);
        enter(&mut mz);
        assert!(
            mz.pending_exits.is_empty(),
            "回车被拿去确认候选了，不该提交：{:?}",
            mz.pending_exits
        );
        let text = mz.input.editor.text();
        assert_eq!(
            text, "/switch fake:model-b",
            "↓ 选中的是第二条，回车把它写回编辑器（实际 `{text}`）"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// `/profile` 的候选也得**真的到得了弹窗**：命令表（形状）+ 花名册
    /// （合法值）两条线都在，才谈得上"接上"。
    #[test]
    fn profile_candidates_reach_the_popup_through_the_zone() {
        let tmp = std::env::temp_dir().join(format!("mypi_prof_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut mz = MainZone::default();
        mz.attach(crate::tui::zone::TermSize { cols: 80, rows: 24 });
        mz.reserved.attach_completion(tmp.clone(), tmp.clone());
        mz.reserved.set_commands(vec![crate::server::wire::CommandInfo {
            name: "/profile".into(),
            aliases: vec![],
            detail: "换 profile".into(),
            args: "profile_name".into(),
            scope: "session".into(),
        }]);
        mz.reserved.set_candidates(
            Vec::new(),
            vec!["default".into(), "novelist".into(), "oh-my-pi".into()],
        );

        type_str(&mut mz, "/profile no");
        let svc = mz.reserved.completion.as_ref().unwrap();
        assert!(
            svc.controller.is_open(),
            "前缀命中一条就该弹窗：{:?}",
            svc.controller
                .popup()
                .items()
                .iter()
                .map(|i| i.name.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(svc.controller.popup().items()[0].name, "novelist");
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn a_leaf_command_submits_on_the_first_enter() {
        let tmp = std::env::temp_dir().join(format!("mypi_leaf_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        let mut mz = MainZone::default();
        mz.attach(crate::tui::zone::TermSize { cols: 80, rows: 24 });
        mz.reserved.attach_completion(tmp.clone(), tmp.clone());
        mz.reserved.set_commands(vec![crate::server::wire::CommandInfo {
            name: "/compact".into(),
            aliases: vec![],
            detail: "压缩历史".into(),
            args: "text".into(),
            scope: "session".into(),
        }]);

        type_str(&mut mz, "/compact");
        assert!(mz.input.hang, "行首斜杠词必须挂起");
        enter(&mut mz);
        match mz.pending_exits.as_slice() {
            [ExitRequest::Submit(text)] => assert_eq!(text, "/compact"),
            other => panic!("回车必须提交，实际 {other:?}"),
        }
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn hang_then_tab_confirms_candidate_into_editor() {
        let tmp = std::env::temp_dir().join(format!("mypi_cpl_{}", std::process::id()));
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        std::fs::write(tmp.join("sub/alpha.txt"), "x").unwrap();
        std::fs::write(tmp.join("sub/beta.md"), "y").unwrap();

        let mut mz = MainZone::default();
        mz.attach(crate::tui::zone::TermSize { cols: 80, rows: 24 });
        mz.reserved
            .attach_completion(tmp.clone(), tmp.clone());

        // "./s" 触发路径补全 + hang
        type_str(&mut mz, "cat ./s");
        assert!(mz.input.hang, "./s 必须挂起");
        // Tab：推进补全（第一次列出/应用前缀）
        tab(&mut mz);
        // 候选应已出现（sub/ 目录）或已写回；再 Tab 确认
        tab(&mut mz);
        let text = mz.input.editor.text();
        assert!(
            text.starts_with("cat ./sub"),
            "Tab 确认后必须写回编辑器，实际: {text:?}"
        );
        std::fs::remove_dir_all(&tmp).ok();
    }
}

#[cfg(test)]
mod anchor_smoke {
    use super::*;
    use crate::tui::zone::{main::MainZone, Zone as _};

    #[test]
    fn path_completion_flow_end_to_end() {
        let tmp = std::env::temp_dir().join(format!("mypi_anchor_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("zeta.rs"), "x").unwrap();

        let mut mz = MainZone::default();
        mz.attach(crate::tui::zone::TermSize { cols: 80, rows: 24 });
        mz.reserved.attach_completion(tmp.clone(), tmp.clone());

        // "cat ./ze" → hang + 弹窗开
        for c in "cat ./ze".chars() {
            mz.deliver(RawEvent::Key {
                key: ratatui::crossterm::event::KeyEvent {
                    code: ratatui::crossterm::event::KeyCode::Char(c),
                    modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
                    kind: ratatui::crossterm::event::KeyEventKind::Press,
                    state: ratatui::crossterm::event::KeyEventState::NONE,
                },
            });
        }
        assert!(mz.input.hang);
        assert!(mz.reserved.completion_open(), "触发词后弹窗必须已开");

        // Tab 确认 → zeta.rs 写回
        mz.deliver(RawEvent::Key {
            key: ratatui::crossterm::event::KeyEvent {
                code: ratatui::crossterm::event::KeyCode::Tab,
                modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
                kind: ratatui::crossterm::event::KeyEventKind::Press,
                state: ratatui::crossterm::event::KeyEventState::NONE,
            },
        });
        assert_eq!(mz.input.editor.text(), "cat ./zeta.rs");
        std::fs::remove_dir_all(&tmp).ok();
    }

    /// 帧级联动：内容变多 → 输入容器长高；弹窗出现 → 保留区按声明拿到
    /// 行数、历史区让位。逐帧仲裁没接上时这条会挂（容器永远两行、
    /// 弹窗永远一行）。
    #[test]
    fn frame_arbitration_reacts_to_content() {
        let tmp = std::env::temp_dir().join(format!("mypi_frame_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        for n in ["zeta.rs", "zeta_one.rs", "zeta_two.rs", "zeta_three.rs", "zeta_four.rs"] {
            std::fs::write(tmp.join(n), "x").unwrap();
        }

        let mut mz = MainZone::default();
        mz.attach(crate::tui::zone::TermSize { cols: 80, rows: 24 });
        mz.reserved.attach_completion(tmp.clone(), tmp.clone());
        let _ = mz.render();
        let idle_input = mz.heights.input;
        assert_eq!(idle_input, 2, "空输入：状态栏 + 底边");

        let press = |mz: &mut MainZone, code: KeyCode, modifiers: KeyModifiers| {
            mz.deliver(RawEvent::Key {
                key: ratatui::crossterm::event::KeyEvent {
                    code,
                    modifiers,
                    kind: ratatui::crossterm::event::KeyEventKind::Press,
                    state: ratatui::crossterm::event::KeyEventState::NONE,
                },
            });
        };
        // 两行输入：容器该长到 3 行
        for c in "hello".chars() {
            press(&mut mz, KeyCode::Char(c), KeyModifiers::NONE);
        }
        press(&mut mz, KeyCode::Enter, KeyModifiers::SHIFT);
        let _ = mz.render();
        assert_eq!(mz.heights.input, 3, "多了一行，容器没长高");

        let history_before_popup = mz.heights.history;
        for c in "cat ./zeta".chars() {
            press(&mut mz, KeyCode::Char(c), KeyModifiers::NONE);
        }
        let frame = mz.render();
        assert!(mz.reserved.completion_open(), "触发词后弹窗没开");
        assert_eq!(mz.heights.reserved, 5, "弹窗申请的行数没兑现");
        assert!(
            mz.heights.history < history_before_popup,
            "弹窗占了位，历史区没让位：{} → {}",
            history_before_popup,
            mz.heights.history
        );
        // 画出来的帧里必须真的有多条候选，而不是被裁到一行
        let all: String = frame
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(all.contains("zeta.rs") && all.contains("zeta_two.rs"), "候选没画全:\n{all}");

        // 硬件光标：落在容器内，且不许进弹窗占的保留区。
        let (x, y) = mz.cursor_position().expect("有输入区就该有光标位");
        assert!(y >= mz.heights.history, "光标跑到历史区里了: {y}");
        assert!(
            y < 24 - mz.heights.reserved,
            "光标进了保留区（弹窗那几行）: y={y} reserved={}",
            mz.heights.reserved
        );
        assert!(x < 80, "列越界: {x}");
        std::fs::remove_dir_all(&tmp).ok();
    }
}
