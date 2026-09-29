//! 节点树 —— 两下 Esc 打开的全屏导航器。
//!
//! 把当前会话所在的**整条分支线**（主干 + 各层 fork）画成一棵时序树：
//! 最新的一块在最下，最古早的在上。用户可以选一块"回到这里"。
//!
//! # 回车不是 fork
//!
//! 回车只做两件**本地**的事：
//! 1. 历史区的显示范围收到那一块为止；
//! 2. 选中的若是用户消息，把它的内容塞回输入框，让用户改。
//!
//! **一个字节都不落盘**。要等用户真的按了发送，才在那一刻 fork/新开
//! session —— 否则树上会冒出没内容的幽灵分支。
//!
//! 渲染规则（几何）在 [`layout`]，画法在 [`render`]。

pub mod layout;
mod render;

use crate::tui::keys::RawEvent;
use crate::tui::zone::{Handover, TermSize, ZoneId};

/// 树上的一行（已算好几何）。一块 = 一行。
pub struct Row {
    pub geom: layout::RowGeom,
    /// 这一行的 `block_id`（回车回到这里的目标）。
    pub block_id: i64,
    /// 显示文本：`user: 前几个字…`。
    pub text: String,
    /// 归属 fork 的 id。
    pub fork: i64,
    /// 角色分类（`user` / `assistant` / `tool_result` / `reasoning` …）。
    /// 树按它上色——四类各占一个色，换主题跟着变。
    pub kind: String,
}

#[derive(Default)]
pub struct TreeZone {
    size: Option<TermSize>,
    rows: Vec<Row>,
    /// 当前高亮（`rows` 下标）。
    pub selected: usize,
    /// 物理行窗口起点（渲染回写）。
    pub scroll: usize,
    /// 回车选中的块（取走一次就清）。**不是分叉**——是"历史区显示到这块为止"。
    return_to: Option<i64>,
}

impl TreeZone {
    /// 进页面：填行，高亮落在当前所在的块。
    pub fn begin(&mut self, rows: Vec<Row>, current: usize) {
        self.rows = rows;
        self.selected = current.min(self.rows.len().saturating_sub(1));
        self.scroll = 0;
    }

    /// 服务端的树到了：算几何、拼行、按**块 id** 找高亮。
    ///
    /// 高亮落在选中的那块**最后一次**出现的行——同块两行（工具往返）时停
    /// 在下面那行更自然。块不在树里（零行状态块）时退到最后一行。
    pub fn on_tree(
        &mut self,
        nodes: &[crate::server::wire::TreeNode],
        forks: &[crate::server::wire::ForkInfo],
        current_block: i64,
    ) {
        let rows = build_rows(nodes, forks);
        let current = rows
            .iter()
            .rposition(|r| r.block_id == current_block)
            .unwrap_or(rows.len().saturating_sub(1));
        self.begin(rows, current);
    }

    /// 取走"用户选了哪一块"（回车）。取走一次就清。
    pub fn take_return_to(&mut self) -> Option<i64> {
        self.return_to.take()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// 当前高亮那一块的 `block_id`。
    pub fn selected_block(&self) -> Option<i64> {
        self.rows.get(self.selected).map(|r| r.block_id)
    }

    fn move_by(&mut self, delta: i64) {
        if self.rows.is_empty() {
            return;
        }
        let last = self.rows.len() - 1;
        let next = self.selected as i64 + delta;
        self.selected = next.clamp(0, last as i64) as usize;
    }
}

/// 把服务端的树拼成可渲染的行。
///
/// 几何（列 / 拐点）全部交给 [`layout`]；这里只把 wire 数据翻译成它要的
/// 形状。
fn build_rows(
    nodes: &[crate::server::wire::TreeNode],
    forks: &[crate::server::wire::ForkInfo],
) -> Vec<Row> {
    let lforks: Vec<layout::ForkRef> = forks
        .iter()
        .map(|f| layout::ForkRef {
            id: f.id,
            parent: f.parent,
            fork_block: f.fork_block,
        })
        .collect();
    let lnodes: Vec<layout::NodeRef> = nodes
        .iter()
        .map(|n| layout::NodeRef {
            block: n.id,
            session: n.session,
        })
        .collect();

    // `rows` 按**树序**返回（分支跟在自己的分叉点后面），所以这里用 `src`
    // 索引回原来的节点，而不是按顺序 zip。
    layout::rows(&lforks, &lnodes)
        .into_iter()
        .map(|p| {
            let n = &nodes[p.src];
            Row {
                geom: p.geom,
                block_id: n.id,
                text: format!("{}: {}", role_label(&n.kind), n.summary),
                fork: n.session,
                kind: n.kind.clone(),
            }
        })
        .collect()
}

/// 角色 → 显示前缀。
fn role_label(kind: &str) -> &'static str {
    match kind {
        "user" => "user",
        "assistant" => "assistant",
        "tool_call" => "tool_call",
        "tool_result" => "tool_result",
        "reasoning" => "reasoning",
        "todo" => "todo",
        "compaction" => "compaction",
        _ => "—",
    }
}

impl crate::tui::zone::Zone for TreeZone {
    fn attach(&mut self, size: TermSize) {
        self.size = Some(size);
    }

    fn on_resize(&mut self, change: crate::tui::zone::SizeChange) {
        self.size = Some(change.to);
    }

    fn deliver(&mut self, event: RawEvent) -> Option<Handover> {
        use ratatui::crossterm::event::{KeyCode, KeyEventKind};
        let RawEvent::Key { key } = event else {
            return None;
        };
        if key.kind != KeyEventKind::Press {
            return None;
        }
        match key.code {
            KeyCode::Up => self.move_by(-1),
            KeyCode::Down => self.move_by(1),
            KeyCode::PageUp => self.move_by(-10),
            KeyCode::PageDown => self.move_by(10),
            KeyCode::Home => self.selected = 0,
            KeyCode::End => self.selected = self.rows.len().saturating_sub(1),
            // 回车：选定"回到这块"。只记下来，交回主区由会话层落实。
            KeyCode::Enter => {
                self.return_to = self.selected_block();
                return Some(Handover {
                    new_owner: ZoneId::Main,
                });
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                return Some(Handover {
                    new_owner: ZoneId::Main,
                });
            }
            _ => {}
        }
        None
    }

    fn render(&mut self) -> Vec<ratatui::text::Line<'static>> {
        // 色号快照每帧抓一次：运行时换主题下一帧生效（和主区同规矩）。
        let t = crate::tui::zone::main::history::render::theme::HistoryTheme::resolve();
        render::rows(&self.rows, self.selected, self.size, &mut self.scroll, &t)
    }
}
