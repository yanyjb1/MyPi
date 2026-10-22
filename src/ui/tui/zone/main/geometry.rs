//! 主区几何 —— 容器边框宽度、输入容器高度上限、视口首次可见行。
//!
//! 主区两个子区共用这一份算术：**输入区**拿它算容器几何（边框占几格、
//! 容器最多几行、光标跟随怎么滚），**保留区**拿它算弹窗宽度上限（弹窗
//! 不得宽过输入框内宽）。都是纯函数，不持有状态，不认调用者。
//!
//! 这里只放**还在被使用**的算术：上一版 `tui/layout.rs` 里那套
//! `Layout`/`compute`/`cursor_y` 是 view.rs 时代的编排残骸（那层编排已
//! 归 Zone 行高仲裁），随本次删除。

/// 边框横向占用的格数：左 `| ` 或 `+-` 占 2 格，右 ` |` 或 `-+` 占 2 格。
///
/// 全项目边框宽度的唯一定义。任何要从宽度里扣掉边框的地方都走它，
/// 数字就不会两边各写一份然后漂开。
pub const BORDER_COLS: usize = 4;

/// 输入容器的行数下限：状态栏 1 行 + 底边 1 行。
pub const MIN_CONTAINER_ROWS: usize = 2;

/// 输入区的换行宽度：终端宽减掉边框。
pub fn inner_width(term_w: u16) -> usize {
    (term_w as usize).saturating_sub(BORDER_COLS).max(1)
}

/// The input container's row ceiling: a fifth of the terminal, at least two rows.
pub fn max_container_height(term_h: u16) -> usize {
    let fifth = (term_h as usize / 5).max(MIN_CONTAINER_ROWS);
    fifth.min(term_h as usize)
}

/// The input container's height: the whole content, capped at a fifth of the terminal
/// and never above the rows available.
///
/// `term_h` 必须是**扣掉保留区之后**的可用高度。
///
/// 下限写成 `2.min(avail)` 而不是结尾补个 `.max(2)`：后者会盖掉前面的
/// `.min(avail)`——只有 1 行的终端上容器会算成 2 行、比屏幕还高，把硬件
/// 光标顶出终端。写成 `2.min(avail)`：有地方就 2 行（状态栏 + 底边），
/// 没地方就有多少算多少。
pub fn container_height(term_h: u16, total_lines: usize) -> usize {
    let avail = term_h as usize;
    (total_lines + 1)
        .min(max_container_height(term_h))
        .min(avail)
        .max(MIN_CONTAINER_ROWS.min(avail))
}

/// 视口能滚到的最大起始行。
///
/// 视口高 `h` 时覆盖的换行行是 `[first, first + h - 2]`（最后一行滑进底边），
/// 覆盖的最后一行不能越过正文末行：`first + h - 2 <= total - 1`。
pub fn max_first(total_lines: usize, container_h: usize) -> usize {
    (total_lines + 1).saturating_sub(container_h)
}

/// 光标跟随滚动：以**最小位移**把光标留在视口内。
///
/// 标准编辑器做法——光标在视口内移动就不滚（画面稳定）；顶到上沿才往上
/// 滚，顶到下沿才往下滚。溢出的输入框里，先前几行会滑出视野，光标往回
/// 走时再滑回来。
///
/// 必须传 `prev`：纯函数分不清「光标本来就在视口中间」和「刚从下面滚
/// 进来」，只靠当前状态会退化成光标永远贴着边。
pub fn adjust_scroll(
    prev: usize,
    total_lines: usize,
    container_h: usize,
    cursor_row: usize,
) -> usize {
    let max_first = max_first(total_lines, container_h);
    let first = prev.min(max_first);
    // 底边那一行也算视口的一部分：光标能到的最后一行是 first + (h-2)
    let last_row_in_view = first + container_h.saturating_sub(2);
    if cursor_row < first {
        cursor_row.min(max_first)
    } else if cursor_row > last_row_in_view {
        (cursor_row - container_h.saturating_sub(2)).min(max_first)
    } else {
        first
    }
}

/// 输入容器内 `(row, col)` → **终端绝对坐标**，带边界钳制。
///
/// 全项目唯一算硬件光标位置的地方。ratatui / crossterm **不做越界检查**
/// ——`Terminal::set_cursor_position` 直接把坐标交给 `MoveTo(x, y)`（见
/// ratatui-core `terminal/render.rs` 的 `apply_buffer_with_cursor`），越界
/// 之后终端怎么反应不归我们管。所以这里是最后一道闸，返回值保证：
///
/// 1. 落在输入容器内（容器行数不足时钳到容器最后一行）；
/// 2. 不越过终端底边；
/// 3. **绝不进保留区**（弹窗那一带）。
///
/// `row` 是**容器内偏移**（0 = 状态栏那一行），不是绝对行。
pub fn cursor_position(
    term: (u16, u16),
    container_y: u16,
    container_rows: u16,
    reserved_rows: u16,
    row: u16,
    col: u16,
) -> (u16, u16) {
    let (cols, rows) = term;
    // 保留区占着最底下 reserved_rows 行，光标最多到它上面一行
    let last_allowed = rows.saturating_sub(reserved_rows).saturating_sub(1);
    let in_container = row.min(container_rows.saturating_sub(1));
    let y = container_y.saturating_add(in_container).min(last_allowed);
    let x = col.min(cols.saturating_sub(1));
    (x, y)
}
