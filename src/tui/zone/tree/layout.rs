//! 会话树的**几何**：X 轴分几列、拐点落在哪一列。
//!
//! 这里只算列，不碰颜色、不碰数据来源——输入是「每条 fork 从哪块岔出」，
//! 输出是「每行块该缩进到第几列」。纯函数，可以单独测。
//!
//! # 形状（用户定稿）
//!
//! ```text
//! │ 主干 A
//! │ 主干 B
//! ├─┐ fork#1
//! │ │ 分1 a
//! │ │ 分1 b
//! │ └─┐ fork#2
//! │   │ 分2 a
//! │ 主干 C
//! ```
//!
//! - 主干 `│` 恒在第 0 列，全程不断——它由 [`trunk_column`] 单独画。
//! - 每条 fork 占一个 X 列，**一列只有一条 fork 在画**（本行所属的那条）。
//! - 分叉行：拐点列画 `├`（贴着自己的轴，从中间岔出去），横线连到子轴列，
//!   子轴列画 `┐`（横在左、竖在下，收住子轴的顶端）。
//! - **一行最多两根 `│`**：主干 + 本行所属 fork 的轴。

/// 一块在树里的位置：时序下标 + 归属 fork。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NodeRef {
    /// 时序下标（`block_id` 排序后的位置）。
    pub index: usize,
    /// 归属会话（fork id）。
    pub fork: i64,
}

/// 一条 fork 的挂载点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForkRef {
    pub id: i64,
    /// 亲会话；`None` = 根（主干）。
    pub parent: Option<i64>,
    /// 从亲会话的哪一块岔出（含）。`None` = 根。
    pub fork_block: Option<i64>,
}

/// 每一列占几个字符（横线 + 一个空格）。渲染层的 `indent_cols` 也读它——
/// 列宽只有这一处定义，几何与画图不会各算各的。
pub const COL_STEP: usize = 3;

/// 每块占的列（X 轴下标）。
///
/// 主干固定 0；一条 fork 的列 = 它的**亲块所在那条 fork 的列** + 1。
///
/// 这样列号就表达了层级：深度 1 的 fork 在第 1 列，深度 2 在第 2 列……而
/// 且**同一层共用一个列号**（不会越走越右）。一列同时只有一条 fork 在画，
/// 所以共列不会撞车——这正是「一行最多两根竖线」成立的原因。
pub fn fork_columns(forks: &[ForkRef]) -> std::collections::HashMap<i64, usize> {
    let mut col: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    // 根：找出 parent 指向不存在的那个（整棵树的根）。
    let ids: std::collections::HashSet<i64> = forks.iter().map(|f| f.id).collect();
    for f in forks {
        if f.parent.map(|p| !ids.contains(&p)).unwrap_or(true) {
            col.insert(f.id, 0);
        }
    }
    // 其余：按"亲 fork 已知"逐层解开。层数不深，直接迭代到不动为止。
    loop {
        let mut grew = false;
        for f in forks {
            if col.contains_key(&f.id) {
                continue;
            }
            let Some(parent) = f.parent else { continue };
            let Some(parent_col) = col.get(&parent).copied() else {
                continue;
            };
            col.insert(f.id, parent_col + 1);
            grew = true;
        }
        if !grew {
            break;
        }
    }
    col
}

/// 每行的几何：缩进到第几列、要不要画分叉。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowGeom {
    /// 本行所属 fork 的 X 列（字符列 = `column * COL_STEP`）。
    pub column: usize,
    /// 本行是不是「某条 fork 从这块岔出去」——分叉行，要画 `├─┐`。
    pub forks_here: bool,
    /// 分叉行的子 fork 列（`forks_here` 才有意义）。
    pub child_column: usize,
}

/// 算每一行画在哪里。
///
/// 返回与 `nodes` 等长的几何表。分叉标记**挂在亲块那一行自己身上**（它
/// 就是那块下面长出来的叉），由渲染层决定画在块下方——几何层只管"这一行
/// 有没有叉、叉到哪一列"。
///
/// `fork_block` 是**块 id**（`blocks.block_id`），不是数组下标：先按
/// `NodeRef::index` 反查它在下标空间的坐标，再挂。
pub fn rows(
    forks: &[ForkRef],
    nodes: &[NodeRef],
    block_id_of: impl Fn(usize) -> i64,
) -> Vec<RowGeom> {
    let col = fork_columns(forks);
    // 块 id → 数组下标。
    let mut at: std::collections::HashMap<i64, usize> = std::collections::HashMap::new();
    for n in nodes {
        at.insert(block_id_of(n.index), n.index);
    }
    // 数组下标 → 在这块下面岔出的那些 fork。
    let mut births: std::collections::HashMap<usize, Vec<i64>> = std::collections::HashMap::new();
    for f in forks {
        if let Some(fb) = f.fork_block
            && let Some(&idx) = at.get(&fb)
        {
            births.entry(idx).or_default().push(f.id);
        }
    }
    for v in births.values_mut() {
        v.sort();
    }

    let mut out = Vec::new();
    for n in nodes {
        let own = col.get(&n.fork).copied().unwrap_or(0);
        let kids = births.remove(&n.index).unwrap_or_default();
        let (forks_here, child_column) = match kids.first() {
            Some(c) => (true, col.get(c).copied().unwrap_or(own + 1)),
            None => (false, 0),
        };
        out.push(RowGeom {
            column: own,
            forks_here,
            child_column,
        });
    }
    out
}
