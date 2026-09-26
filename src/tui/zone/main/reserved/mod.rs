//! 保留子区 —— 声明行高的源头：被借用时声明更大行高，不消费广播。

use super::SubZone;

pub mod area;
pub mod completion;

/// 保留区子区。内部是 claim 注册表（components::reserved::ReservedArea）
/// 和一个补全服务（可被激活、可借键的保留区住户）。
pub struct ReservedZone {
    pub area: crate::tui::zone::main::reserved::area::ReservedArea,
    /// 当前声明的行数（借用方申报；空闲 = 1）。
    pub want: u16,
    /// 补全服务：保留区第一个住户。挂上即入册（claims 同步登记）。
    pub completion: Option<crate::tui::zone::main::reserved::completion::CompletionService>,
}

impl Default for ReservedZone {
    fn default() -> Self {
        Self {
            area: crate::tui::zone::main::reserved::area::ReservedArea::default(),
            want: crate::tui::zone::main::reserved::area::IDLE_ROWS,
            completion: None,
        }
    }
}

impl ReservedZone {
    /// 帧首结算：登记持有者 + 冻结行高，再把冻结值当成本子区的申请值。
    ///
    /// 行高的**唯一出口**是 `area.rows(term_h)`：谁想占位谁去 `Claim::want`
    /// 声明，`area` 冻结，本子区只负责把冻结值报给 Zone。空闲时是 1 行，
    /// 所以没有持有者时布局不会跳。
    pub fn settle(&mut self, term: crate::tui::zone::TermSize) {
        let mut claims: Vec<&mut dyn area::Claim> = Vec::new();
        if let Some(c) = self.completion.as_mut() {
            claims.push(c as &mut dyn area::Claim);
        }
        self.area.resolve(&mut claims);
        self.want = self.area.rows(term.rows);
    }

    /// 借键投递：把输入区翻译好的动作交给活动持有者，按其
    /// `Claim::accepts()` 声明自取；无人接或未声明则退回（false）。
    /// 输入区对这里零知识——它不知道谁持有、接了干什么。
    pub fn lend(&mut self, action: &crate::tui::zone::main::input::semantics::Action) -> bool {
        let mut claims: Vec<&mut dyn crate::tui::zone::main::reserved::area::Claim> = Vec::new();
        if let Some(c) = self.completion.as_mut() {
            claims.push(c as &mut dyn crate::tui::zone::main::reserved::area::Claim);
        }
        // 投递前先**重新认一遍持有者**：持有者是上一帧 `settle` 时认出来的，
        // 而"弹窗刚开、同一个事件批次里又来一个 ↑↓"（手快、或按键被合并）
        // 是完全可能的——那时账本还停在上一帧（无人持有），这一下 ↑↓ 就被
        // 丢掉了。行高按上一帧的值走，下一帧 `settle` 再修正。
        self.area.resolve(&mut claims);
        self.area.offer(action, &mut claims)
    }

    /// 模型与 profile 的补全候选（启动时一次；见 `CompletionService`）。
    pub fn set_candidates(
        &mut self,
        models: Vec<crate::tui::zone::main::reserved::completion::controller::ModelCandidate>,
        profiles: Vec<String>,
    ) {
        if let Some(c) = self.completion.as_mut() {
            c.set_candidates(models, profiles);
        }
    }

    pub fn set_commands(&mut self, commands: Vec<crate::server::wire::CommandInfo>) {
        if let Some(c) = self.completion.as_mut() {
            c.set_commands(commands);
        }
    }

    pub fn attach_completion(&mut self, cwd: std::path::PathBuf, home: std::path::PathBuf) {
        self.completion = Some(crate::tui::zone::main::reserved::completion::CompletionService::new(cwd, home));
    }

    /// 输入区 Confirm 入口：Tab/Enter 推进补全（开弹窗/前缀扩展/确认
    /// 由 on_tab 内部处理）。返回要写回编辑器的替换；None = 无候选可推进
    /// （输入区把这个键还给编辑器自己解释）。
    pub fn confirm(
        &mut self,
        text: &str,
        cursor: usize,
    ) -> Option<crate::tui::zone::main::reserved::completion::engine::CompletionAction> {
        use crate::tui::zone::main::reserved::completion::engine::CompletionAction;
        let svc = self.completion.as_mut()?;
        match svc.on_tab(text, cursor) {
            CompletionAction::None => None,
            r => Some(r),
        }
    }

    /// 输入区每次内容/光标变化后的候选刷新（col = 光标终端绝对列）。
    pub fn refresh_completion(&mut self, text: &str, cursor: usize, col: usize) {
        if let Some(svc) = self.completion.as_mut() {
            svc.anchor_col = col;
            svc.refresh(text, cursor);
        }
    }

    /// 弹窗是否开着（保留区行高声明依据）。
    pub fn completion_open(&self) -> bool {
        self.completion
            .as_ref()
            .is_some_and(|s| s.controller.is_open())
    }
}

impl SubZone for ReservedZone {
    fn request_height(&self, _term: crate::tui::zone::TermSize) -> u16 {
        self.want
    }

    fn assign(&mut self, _term: crate::tui::zone::TermSize, rows: u16) {
        self.want = rows.max(crate::tui::zone::main::reserved::area::IDLE_ROWS);
    }

    fn on_height_changed(&mut self, _rows: u16) -> bool {
        false // 行高声明源头，不消费广播
    }

    /// 声明：无。保留区不需要键位触发；它的事件由输入区联动唤起。
    fn accepts(&self) -> &'static [crate::tui::zone::RawEventKind] {
        &[]
    }

    fn deliver(&mut self, _event: &crate::tui::zone::RawEvent) -> bool {
        false // 未声明任何事件；输入区联动走自己的接口，不过这里
    }
}

impl ReservedZone {
    /// 自渲染：空闲画一行空行；有持有者时按冻结行高交给持有者画。
    pub fn render_rows(
        &mut self,
        term_w: u16,
        p: &crate::tui::theme::Palette,
    ) -> Vec<ratatui::text::Line<'static>> {
        let Some(id) = self.area.holder() else {
            return vec![ratatui::text::Line::from("")];
        };
        // 行高用仲裁发下来的（= settle 时 area 冻结的那个值）；
        // 注意不是 term_w：那是宽度，行高跟宽度没关系。
        let rows = usize::from(self.want).max(1);
        // 持有者即补全服务（现阶段唯一住户）：按锚点列画。
        if id == "completion"
            && let Some(svc) = self.completion.as_ref()
        {
            use crate::tui::zone::main::reserved::area::Claim as _;
            let mut lines = svc.draw(term_w, rows, p);
            let anchor = svc.anchor_col;
            // 锚点缩进：每行左侧补 anchor 列空格（弹窗自身的左移规则
            // 已在 popup::render 里算好，这里只平移）。
            if anchor > 0 {
                for l in lines.iter_mut() {
                    let mut spans = vec![ratatui::text::Span::raw(" ".repeat(anchor))];
                    spans.append(&mut l.spans);
                    l.spans = spans;
                }
            }
            return lines;
        }
        vec![ratatui::text::Line::from("")]
    }
}
