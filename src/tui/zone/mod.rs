//! Zone 契约 —— 全部权责的规范声明。破坏性重构的宪法。
//!
//! 层级：APP → Zone → 子区（SubZone）→ 组件。
//!
//! # APP（Router）
//!
//! APP 只做四件事，无一例外：
//! 1. 接入外部事件（键鼠/粘贴/session 事件），归一化为 [`Action`]。
//! 2. 把键鼠所有权**下发给当前所有者**——不路由、不仲裁、不排优先级。
//! 3. 登记所有权交接：Zone 之间交接必须向 APP 注册，APP 始终知道当前所有者。
//! 4. 尺寸契约：启动时把宽高**传一次**；此后只在终端 resize 时**广播**
//!    通知，Zone 自己消化，APP 不重切、不仲裁、不碰内容。
//!
//! APP 不渲染任何东西，不知道任何 Zone 的内部状态。
//!
//! # Zone
//!
//! - 接过 APP 传来的尺寸，**自己仲裁自己的行高分配**。
//! - 所有者交接（Zone A → Zone B）通过 [`Handover`] 登记；Zone 彼此
//!   **互斥**：同一时刻只有一个所有者，不存在优先级，不存在竞态。
//! - 渲染：Zone 自己渲染自己。APP 不渲染任何东西。
//!
//! # 子区（MainZone 特有，规矩固化可复用）
//!
//! 子区向 Zone **申请行高**，Zone 分配，子区拿分配到的行高自己决定渲染内容。
//! 行高变更时 Zone **广播通知**所有订阅的子区：
//!
//! - 保留区被借用时可声明更大的行高，获批后触发广播；
//! - 历史区订阅广播，可用行高随之收缩；
//! - 输入区自限行高，不订阅广播。
//!
//! 本文件只放契约（trait + 数据类型）。任何具体 Zone 的实现一律放
//! 兄弟模块（`main.rs` 等），此处的规矩永不被实例污染。

pub use crate::tui::keys::RawEvent;

/// 终端尺寸（列，行）。APP 在启动时测量一次，resize 时广播新值。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermSize {
    pub cols: u16,
    pub rows: u16,
}

/// resize 通知：旧尺寸 → 新尺寸。Zone 自行决定如何消化，APP 不追问。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeChange {
    pub from: TermSize,
    pub to: TermSize,
}

/// 行高广播：Zone 完成子区行高分配后发出，携带受影响的子区新行高。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeightNotice {
    pub rows: u16,
}

/// Zone 身份。互斥：同一时刻至多一个 Zone 持有所有权。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneId {
    /// 主区：历史 + 输入 + 保留的宿主。
    Main,
    /// /resume 会话选择器。全屏接管，激活时持有所有权。
    Resume,
    /// 会话树导航器。全屏接管，激活时持有所有权。
    Tree,
}

/// Zone 契约。APP 只通过这四个方法与 Zone 说话。
pub trait Zone {
    /// APP 移交启动尺寸。Zone 据此初始化自己的布局状态。
    fn attach(&mut self, size: TermSize);

    /// 终端 resize 广播。Zone 自己决定内部如何处理。
    fn on_resize(&mut self, change: SizeChange);

    /// 处理下发的事件（键/粘贴/滚轮，见 [`RawEvent`]）。返回
    /// `Some(new_owner)` 表示所有权移交给该 Zone，APP 必须登记。
    /// Zone 之间互斥：交接是唯一的通信方式，没有优先级。
    /// 事件的语义解释完全是 Zone 的事——APP 只捕捉、不下发含义。
    fn deliver(&mut self, event: RawEvent) -> Option<Handover>;

    /// 自渲染。行高由 Zone 自己的仲裁决定，内容自己画。
    /// `&mut`：渲染会写自己的缓存（块缓存 admit、窗口游标），这是
    /// Zone 的私有状态，不是对外可变性的泄露。
    fn render(&mut self) -> Vec<ratatui::text::Line<'static>>;
}

/// 交接结果：新所有者。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Handover {
    pub new_owner: ZoneId,
}

/// APP 侧的所有权账本。Zone 交接必须过这里，账本永远反映当前所有者。
#[derive(Debug)]
pub struct Ownership {
    current: ZoneId,
}

impl Ownership {
    pub fn new(initial: ZoneId) -> Self {
        Self { current: initial }
    }

    /// 当前所有者。
    pub fn current(&self) -> ZoneId {
        self.current
    }

    /// 登记一次交接。
    pub fn register(&mut self, h: Handover) {
        self.current = h.new_owner;
    }
}

/// 子区规范：向 Zone 申请行高，拿分配结果自渲染；**显式声明自己
/// 解释哪些物理事件**。
///
/// - 行高：Zone 是分配者，子区是申请者；分配结果通过 [`SubZone::assign`]
///   回给子区，随后的 [`SubZone::on_height_changed`] 是 Zone 的行高广播——
///   订阅与否是子区的自由（历史区订阅，输入区不订阅）。
/// - 事件：Zone 从 APP 接过全部 [`RawEvent`]，只转发给**声明了要解释
///   对应事件的子区**；声明是子区注册时的一部分，Zone 不猜、不默认。
///   子区拿到事件后翻译成自己的内部事件/动作，怎么做是子区内部的事。
pub trait SubZone {
    /// 向 Zone 申请行高：返回本子区当前需要的行数。Zone 据此仲裁。
    ///
    /// `term` 是终端尺寸：**申请者需要知道终端才能算出自己要多少行**
    /// （输入区按内容换行后算容器高、上限是终端的 1/4）。不给尺寸，
    /// 子区就只能把上一次的分配结果当申请值回读，永远长不大。
    fn request_height(&self, term: TermSize) -> u16;

    /// Zone 的分配结果。子区收到后自行调整内部状态。
    ///
    /// 一并给终端尺寸：子区要拿宽度做换行/命中（键位在两次 resize 之间
    /// 随时到达，不能等到下一次渲染才知道多宽）。
    fn assign(&mut self, term: TermSize, rows: u16);

    /// 行高广播：Zone 通知订阅者可用行高变了。返回是否消费本次通知。
    fn on_height_changed(&mut self, rows: u16) -> bool;

    /// 显式声明本子区解释哪些物理事件。未声明的事件 Zone 不下发。
    /// 声明的是"要不要"，不是"怎么处理"——处理在子区自己手里。
    fn accepts(&self) -> &'static [RawEventKind];

    /// 分发入口：Zone 把声明过的事件**广播**进来（同一次事件会到达
    /// 每一个声明了该类别的子区）。子区自己解释、自己决定是否反应；
    /// 返回值只做诊断，Zone 不拿它做路由——消费与否是子区内部的事。
    fn deliver(&mut self, event: &RawEvent) -> bool;
}

/// [`RawEvent`] 的静态形状：子区声明的"我解释哪类事件"。
///
/// 声明粒度是事件类别，不是具体键位——接受 `Key` 的子区拿到全部键
/// （修饰键原样携带），具体哪个键触发什么行为是子区内部的解释。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawEventKind {
    /// 键盘。
    Key,
    /// 粘贴。
    Paste,
    /// 滚轮向上。
    ScrollUp,
    /// 滚轮向下。
    ScrollDown,
}



pub mod main;
pub mod resume;

pub use main::MainZone;
pub use resume::ResumeZone;
