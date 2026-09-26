//! 字形表 —— ASCII 预设。
//!
//! 取值照抄 omp `theme/symbols.ts` 的 `ASCII_SYMBOLS`（我们的主题 JSON
//! 也声明 `"preset": "ascii"`）。字形与颜色分开：这里只放「长什么样」，
//! 颜色仍由 [`super::theme`] 决定。
//!
//! 现在只有工具卡用得到的一小撮；等 unicode/nerd 预设真要支持时，
//! 这张表就是挂载点（omp 的 `Theme.symbols` 是同一位置）。

/// 成功。
pub(super) const OK: &str = "[ok]";
/// 失败。
pub(super) const ERR: &str = "[!!]";
/// 还没开始 / 等结果（spinner 接进来之前，进行中都用它）。
pub(super) const PENDING: &str = "[*]";

/// 展开提示（收起且有隐藏行时才出现）。
pub(super) const EXPAND_HINT: &str = "[ctrl+o: Expand]";

/// meta 各段之间的分隔符。
pub(super) const SEP: &str = "·";

// ---- 任务清单（`todo` 工具）----
// 五个状态各一个字形，与 `TodoStatus` 一一对应（ASCII 预设，和上面的工具字形同族）。
/// 还没开始。
pub(super) const TODO_PENDING: &str = "[ ]";
/// 正在做（一次只该有一项）。
pub(super) const TODO_ACTIVE: &str = "[>]";
/// 已完成。
pub(super) const TODO_DONE: &str = "[x]";
/// 卡住了（卡片里会附原因）。
pub(super) const TODO_BLOCKED: &str = "[!]";
/// 放弃（与"还没做"不是一回事）。
pub(super) const TODO_DROPPED: &str = "[-]";

// ---- 树形连接符（任务清单那块用）----
//
// 与 omp 的 ASCII 预设逐字相同（`theme/symbols.ts` 的 ascii 段：
// `tree.branch` / `tree.last` / `tree.vertical` / `tree.hook`）。

/// 非末项：`|-- `。
pub(super) const TREE_BRANCH: &str = "|--";

/// 末项：`'-- `。
pub(super) const TREE_LAST: &str = "'--";

/// 竖线（延续行用）。
pub(super) const TREE_VERTICAL: &str = "|";

/// 收口：`` `- ``（补横线成 `` `---- ``）。
pub(super) const TREE_HOOK: &str = "`-";
