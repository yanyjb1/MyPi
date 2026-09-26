//! APP —— Router。只做四件事：
//! 1. 接入外部事件，归一化。
//! 2. 键鼠所有权下发给当前所有者。
//! 3. 登记所有权交接。
//! 4. 启动传一次尺寸；resize 时广播。
//!
//! 不渲染、不路由优先级、不仲裁行高、不碰任何 Zone 内部。

use crate::tui::zone::RawEvent;
use crate::tui::zone::{Ownership, SizeChange, TermSize, Zone, ZoneId};

/// APP 侧账本 + Zone 注册表。APP 知道所有 Zone 的存在，
/// 不知道它们内部的一个字段。
pub struct App {
    /// 所有权账本：当前所有者。交接必过这里。
    pub ownership: Ownership,
    /// 主区 Zone（历史 + 输入 + 保留的宿主）。
    pub main: crate::tui::zone::MainZone,
    /// `/resume` 的全屏会话选择器。常驻（不每次 new），进页面时刷新。
    pub resume: crate::tui::zone::ResumeZone,
    /// 会话出口请求队列（Submit/Interrupt/Quit）。deliver 后由事件
    /// 循环 `take_outcomes` 取走处理。
    outcomes: Vec<crate::tui::zone::main::input::ExitRequest>,
}

impl App {
    /// 启动：把初始尺寸交给启动 Zone（尺寸契约的"传一次"），
    /// 并把它登记为初始所有者。
    ///
    /// 启动哪个 Zone 由调用方决定：命令行直接进 → `ZoneId::Main`；
    /// 未来 `cli --resume` → `ZoneId::Resume`（Zone 实现后挂上即可，
    /// APP 代码不变）。
    pub fn new(size: TermSize, startup: ZoneId) -> Self {
        let mut main = crate::tui::zone::MainZone::default();
        main.attach(size);
        let mut resume = crate::tui::zone::ResumeZone::default();
        resume.attach(size);
        Self {
            ownership: Ownership::new(startup),
            main,
            resume,
            outcomes: Vec::new(),
        }
    }

    /// 进 `/resume` 页面：交接所有权，并让页面去拉列表。
    ///
    /// 交接必过账本（Zone 互斥，APP 永远知道当前所有者）；拉列表是
    /// 页面自己声明的事，主循环随后取走执行。
    pub fn open_resume(&mut self, cwd: std::path::PathBuf) {
        self.ownership.register(crate::tui::zone::Handover {
            new_owner: ZoneId::Resume,
        });
        self.resume.begin(cwd);
    }

    /// 取走 resume 页面要主循环替它做的事（发列表请求 / 附着）。
    pub fn take_resume_requests(&mut self) -> Vec<crate::tui::zone::resume::Request> {
        self.resume.take_requests()
    }

    /// 终端 resize：广播给所有 Zone，谁怎么消化是 Zone 的私事。
    pub fn on_resize(&mut self, to: TermSize) {
        // 广播给所有 Zone（各自的旧尺寸自己记得）。
        let from = self.main.size.unwrap_or(to);
        self.main.on_resize(SizeChange { from, to });
        let from = self.resume.size().unwrap_or(to);
        self.resume.on_resize(SizeChange { from, to });
    }

    /// 归一化动作下发：按账本给当前所有者。返回 Some 表示发生了
    /// 交接，登记。
    pub fn deliver(&mut self, event: RawEvent) {
        let owner = self.ownership.current();
        let handover = match owner {
            ZoneId::Main => self.main.deliver(event),
            ZoneId::Resume => self.resume.deliver(event),
            ZoneId::Tree => None, // 会话树 Zone 未实现
        };
        if let Some(h) = handover {
            self.ownership.register(h);
        }
        // 出口请求逐级上报的最后一级：Zone → APP。
        if let Some(x) = self.main.pending_exit.take() {
            self.outcomes.push(x);
        }
    }

    /// 借出主区给 SessionView 消费服务端推送（同一循环内，借用不跨界），
    /// 用完 `app.main_mut()` 写回。避免 move：Zone 大，且渲染还要它。
    pub fn main_mut(&mut self) -> &mut crate::tui::zone::MainZone {
        &mut self.main
    }

    /// 取走全部会话出口请求（每次 deliver 至多一条，但取空更稳）。
    pub fn take_outcomes(&mut self) -> Vec<crate::tui::zone::main::input::ExitRequest> {
        std::mem::take(&mut self.outcomes)
    }

    /// 当前所有者（诊断用）。
    pub fn current_owner(&self) -> ZoneId {
        self.ownership.current()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_starts_with_main_as_owner() {
        let app = App::new(TermSize { cols: 80, rows: 24 }, ZoneId::Main);
        assert_eq!(app.current_owner(), ZoneId::Main);
    }

    #[test]
    fn resize_broadcasts_to_zone() {
        let mut app = App::new(TermSize { cols: 80, rows: 24 }, ZoneId::Main);
        app.on_resize(TermSize { cols: 100, rows: 30 });
        assert_eq!(app.main.size, Some(TermSize { cols: 100, rows: 30 }));
    }
}
