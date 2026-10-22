//! 会话树的**几何**：每行缩进几层、连到哪个兄弟、哪些岔路口要竖线。
//!
//! 形状照抄 pi 的 `tree-selector`（用户定："既然要抄作业，就抄人家的"）。
//! 规则三条：
//!
//! 1. **单链不缩进**：一个父只有一个孩子时，孩子跟它同列——主干从头到尾
//!    齐平在左，不会越走越右。
//! 2. **有岔路才缩进**：父有多个孩子，这些孩子全部缩进一层。
//! 3. **兄弟用 `├─` / `└─` 分**：祖先的岔路口竖一条 `│` 垂到那一支结束。
//!
//! ```text
//! 主干 A
//! 主干 B
//! ├─ 分1 a
//! │  分1 b
//! └─ 分2 a
//! 主干 C
//! ```
//!
//! 每层占 [`COL_STEP`] 个字符。几何是纯函数，可以单独测。
//!
//! # 输入为什么是平铺的
//!
//! 服务端给的是**按时序**（`block_id`）排的整棵树的块，不是树。这里把它还原
//! 成树：同会话内前一块是后一块的父；一条 fork 的第一块，父是它 `fork_block`
//! 指的那一块。因为父一定排在子前面（分叉点早于分支自己的块），一趟扫描就能
//! 挂好，不用递归查找。
//!
//! 平铺回行时按**树序**（先序），不是时序——分支的内容要跟在它分叉的地方，
//! 不能跟主干后来的块混在一起。

use std::collections::HashMap;

/// 每层缩进几个字符（`├` `─` 空格）。
pub const COL_STEP: usize = 3;

/// 时序平铺里的一个节点（一行）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeRef {
    /// `block_id`。
    pub block: i64,
    /// 归属会话。
    pub session: i64,
}

/// 一条 fork 的挂载点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForkRef {
    pub id: i64,
    pub parent: Option<i64>,
    pub fork_block: Option<i64>,
}

/// 一行的几何。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RowGeom {
    /// 缩进层数（每层 [`COL_STEP`] 列）。
    pub indent: usize,
    /// 连接符：`Some(true)` = 最后一个兄弟（`└─`），`Some(false)` = 后面还有
    /// 兄弟（`├─`），`None` = 不画（根，或单链上的节点）。
    pub connector: Option<bool>,
    /// 要竖线的层：(层号, 画不画)。不画也占位——对齐靠它。
    pub gutters: Vec<(usize, bool)>,
}

impl RowGeom {
    /// 前缀占多少列。
    pub fn prefix_cols(&self) -> usize {
        self.indent * COL_STEP
    }

    /// 第 `level` 层要不要竖线（`None` = 那层不画结构线，留白）。
    pub fn gutter(&self, level: usize) -> Option<bool> {
        self.gutters
            .iter()
            .find(|(l, _)| *l == level)
            .map(|(_, show)| *show)
    }

    /// 连接符落在第几层（它总在本行缩进的**上一层**，和它的父同列）。
    pub fn connector_level(&self) -> Option<usize> {
        self.connector.map(|_| self.indent.saturating_sub(1))
    }
}

/// 树上的一行：来自时序里的第几个节点 + 它的几何。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    pub src: usize,
    pub geom: RowGeom,
}

/// 把时序平铺的一串节点还原成树，再按树序平铺回来，附每行几何。
///
/// 一行 = 条目，不一定是块：同一块的两条（一次工具往返）按出现顺序串成链，
/// 所以"分叉点"落在的是那一块的**最后一行**，分支接在它下面。
pub fn rows(forks: &[ForkRef], nodes: &[NodeRef]) -> Vec<Placed> {
    let n = nodes.len();
    let fork_of: HashMap<i64, Option<i64>> = forks.iter().map(|f| (f.id, f.fork_block)).collect();

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut roots: Vec<usize> = Vec::new();
    // 每块**最后一行**的下标：分叉点和同会话后继都挂在它下面。
    let mut block_last: HashMap<i64, usize> = HashMap::new();
    let mut session_prev_block: HashMap<i64, i64> = HashMap::new();

    for (i, node) in nodes.iter().enumerate() {
        let parent = if let Some(&prev_row) = block_last.get(&node.block) {
            // 同一块的下一条目（一次工具往返的两半）。
            Some(prev_row)
        } else if let Some(&prev_block) = session_prev_block.get(&node.session) {
            // 同会话里的前一块。
            block_last.get(&prev_block).copied()
        } else {
            // 本会话的第一块：挂在父会话的分叉点上（分叉点可能在祖先前缀里，
            // 也可能就在父会话自己的块里，两种都靠 block_last 反查）。
            fork_of
                .get(&node.session)
                .copied()
                .flatten()
                .and_then(|fb| block_last.get(&fb).copied())
        };
        match parent {
            Some(p) => children[p].push(i),
            None => roots.push(i),
        }
        block_last.insert(node.block, i);
        session_prev_block.insert(node.session, node.block);
    }

    // 先序平铺。栈元素：节点、缩进层数、连接符、本行要竖的线。
    let mut out: Vec<Placed> = Vec::with_capacity(n);
    let mut stack: Vec<(usize, usize, Option<bool>, Vec<(usize, bool)>)> = Vec::new();
    for &r in roots.iter().rev() {
        stack.push((r, 0, None, Vec::new()));
    }
    while let Some((idx, indent, connector, gutters)) = stack.pop() {
        let kids = &children[idx];
        let multi = kids.len() > 1;
        // 有岔路才缩进：单链跟父同列，主干永远齐平。
        let child_indent = if multi { indent + 1 } else { indent };
        // 本行带了连接符，它下面这一支的竖线就从它那一层垂下来；不是最后一个
        // 兄弟才画线（最后一支左边是空的）。
        let child_gutters = match connector {
            Some(is_last) => {
                let mut g = gutters.clone();
                g.push((indent.saturating_sub(1), !is_last));
                g
            }
            // 单链不新增竖线：孩子继承原样。
            None => gutters.clone(),
        };
        out.push(Placed {
            src: idx,
            geom: RowGeom {
                indent,
                connector,
                gutters,
            },
        });
        for (k, &child) in kids.iter().enumerate().rev() {
            let is_last = k + 1 == kids.len();
            stack.push((
                child,
                child_indent,
                if multi { Some(is_last) } else { None },
                child_gutters.clone(),
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一条主干 + 两条分支：主干 A B，B 后岔出 [分1 a, 分1 b]，再岔出 [分2 a]，
    /// 主干接着 C D。
    fn sample() -> (Vec<ForkRef>, Vec<NodeRef>) {
        let forks = vec![
            ForkRef {
                id: 1,
                parent: None,
                fork_block: None,
            },
            ForkRef {
                id: 2,
                parent: Some(1),
                fork_block: Some(2),
            },
            ForkRef {
                id: 3,
                parent: Some(2),
                fork_block: Some(4),
            },
        ];
        let spec: Vec<(i64, i64)> = vec![(1, 1), (2, 1), (3, 2), (4, 2), (5, 3), (6, 1), (7, 1)];
        let nodes = spec
            .into_iter()
            .map(|(block, session)| NodeRef { block, session })
            .collect();
        (forks, nodes)
    }

    fn lines(forks: &[ForkRef], nodes: &[NodeRef]) -> Vec<(usize, String)> {
        rows(forks, nodes)
            .into_iter()
            .map(|p| {
                let g = &p.geom;
                let mut s = String::new();
                for i in 0..g.prefix_cols() {
                    let level = i / COL_STEP;
                    let pos = i % COL_STEP;
                    if let Some(show) = g.gutter(level) {
                        s.push(if pos == 0 && show { '│' } else { ' ' });
                    } else if g.connector_level() == Some(level) {
                        s.push(match pos {
                            0 => {
                                if g.connector == Some(true) {
                                    '└'
                                } else {
                                    '├'
                                }
                            }
                            1 => '─',
                            _ => ' ',
                        });
                    } else {
                        s.push(' ');
                    }
                }
                s.push_str(&format!("块{}", nodes[p.src].block));
                (p.src, s)
            })
            .collect()
    }

    /// 主干齐平：一条直线上的节点缩进**全程为 0**，正文不会越走越右。
    #[test]
    fn a_single_child_chain_never_indents() {
        let (forks, nodes) = sample();
        let out = rows(&forks, &nodes);
        // 块 1、2 是主干；块 6、7 挂在分叉点上，也缩进一层——但主干自己那一段
        // 必须齐平。
        let trunk: Vec<usize> = out
            .iter()
            .filter(|p| nodes[p.src].session == 1 && matches!(nodes[p.src].block, 1 | 2))
            .map(|p| p.geom.indent)
            .collect();
        assert_eq!(trunk, vec![0, 0], "主干不许缩进");
    }

    /// 岔路口才缩进，且两支出自同一个父：一条 `├─`、一条 `└─`。
    #[test]
    fn a_branch_indents_once_and_gets_a_connector() {
        let (forks, nodes) = sample();
        let out = rows(&forks, &nodes);
        let of = |block: i64| {
            out.iter()
                .find(|p| nodes[p.src].block == block)
                .unwrap()
                .geom
                .clone()
        };
        assert_eq!(of(3).indent, 1, "分支第一块缩进一层");
        assert_eq!(of(3).connector, Some(false), "后面还有兄弟");
        assert_eq!(of(6).connector, Some(true), "最后一条分支用 └─");
        assert_eq!(of(4).indent, 1, "分支内部跟分支头同列");
        assert_eq!(of(5).connector, None, "独苗不画连接符");
    }

    /// 分支的内部竖线一直垂到那一支结束：`分1 b` 左边还得有 `│`。
    #[test]
    fn a_gutter_runs_until_the_branch_ends() {
        let (forks, nodes) = sample();
        let out = rows(&forks, &nodes);
        let of = |block: i64| {
            out.iter()
                .find(|p| nodes[p.src].block == block)
                .unwrap()
                .geom
                .clone()
        };
        assert_eq!(of(4).gutter(0), Some(true), "非末支的内部要竖线");
        assert_eq!(of(7).gutter(0), Some(false), "末支的左边留白");
    }

    /// 平铺按**树序**：分支的块跟在一起，不按时钟散进主干里。
    #[test]
    fn rows_follow_the_tree_not_the_clock() {
        let (forks, nodes) = sample();
        let order: Vec<i64> = rows(&forks, &nodes)
            .into_iter()
            .map(|p| nodes[p.src].block)
            .collect();
        assert_eq!(order, vec![1, 2, 3, 4, 5, 6, 7]);
        // 真反例：分支写完一块，主干接着写，分支又写。时序是 3 4 5，
        // 树序必须是 3 5 4——分支的两块跟在一起。
        let nodes2: Vec<NodeRef> = vec![
            NodeRef {
                block: 1,
                session: 1,
            },
            NodeRef {
                block: 2,
                session: 1,
            },
            NodeRef {
                block: 3,
                session: 2,
            },
            NodeRef {
                block: 4,
                session: 1,
            },
            NodeRef {
                block: 5,
                session: 2,
            },
        ];
        let forks2 = vec![
            ForkRef {
                id: 1,
                parent: None,
                fork_block: None,
            },
            ForkRef {
                id: 2,
                parent: Some(1),
                fork_block: Some(2),
            },
        ];
        let order2: Vec<i64> = rows(&forks2, &nodes2)
            .into_iter()
            .map(|p| nodes2[p.src].block)
            .collect();
        assert_eq!(order2, vec![1, 2, 3, 5, 4], "分支插在分叉点后面");
    }

    /// 渲染出来的前缀长什么样（这一层只做几何，画法是 render 的事，这里
    /// 按同一套规则拼一遍，锁住形状）。
    #[test]
    fn the_shape_is_the_one_we_agreed_on() {
        let (forks, nodes) = sample();
        let got: Vec<String> = lines(&forks, &nodes).into_iter().map(|(_, s)| s).collect();
        assert_eq!(
            got,
            vec![
                "块1",
                "块2",
                "├─ 块3",
                "│  块4",
                "│  块5",
                "└─ 块6",
                "   块7",
            ]
        );
    }
}
