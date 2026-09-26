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

/// 输入容器的高度上限：终端高度的 1/4，至少 2 行，且不超过终端本身。
pub fn max_container_height(term_h: u16) -> usize {
    let quarter = (term_h as usize / 4).max(MIN_CONTAINER_ROWS);
    quarter.min(term_h as usize)
}

/// 输入容器高度：想装下全部内容（状态栏 + 正文行 + 底边 = 行数 + 1），
/// 但不超过 1/4 终端高，也**不超过可用高度**。
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
pub fn adjust_scroll(prev: usize, total_lines: usize, container_h: usize, cursor_row: usize) -> usize {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 容器绝不能高过可用高度：结尾补 `.max(2)` 会把 1 行终端算成 2 行，
    /// 比屏幕还高。这条是那个坑的回归闸。
    #[test]
    fn container_never_exceeds_the_viewport() {
        for term_h in 0u16..=40 {
            for lines in [0usize, 1, 3, 50] {
                let h = container_height(term_h, lines);
                assert!(
                    h <= (term_h as usize).max(MIN_CONTAINER_ROWS.min(term_h as usize)),
                    "容器 {h} 高过可用高度 {term_h}（{lines} 行内容）"
                );
            }
        }
    }

    /// 单行输入占两行（状态栏 + 底边）；多行内容把容器撑高。
    #[test]
    fn container_grows_with_content_up_to_a_quarter() {
        assert_eq!(container_height(24, 1), 2, "单行输入两行就够");
        assert_eq!(container_height(24, 3), 4);
        // 可用 23 → 上限 23/4 = 5；内容再多也只给 5
        assert_eq!(container_height(23, 100), 5);
        assert_eq!(max_container_height(24), 6);
        assert_eq!(max_container_height(40), 10);
    }

    /// 光标跟随：视口内不动，顶到边界才滚，且不越过正文首行。
    #[test]
    fn scroll_follows_the_cursor_with_minimum_displacement() {
        // 视口高 4（覆盖 3 行），共 10 行
        let h = 4;
        assert_eq!(adjust_scroll(0, 10, h, 1), 0, "视口内不该滚");
        assert_eq!(adjust_scroll(0, 10, h, 4), 2, "顶到下沿该往下滚");
        assert_eq!(adjust_scroll(5, 10, h, 2), 2, "光标跳到上面该往上滚");
        // 正文很短时滚不动
        assert_eq!(adjust_scroll(9, 2, h, 0), max_first(2, h));
    }

    /// 光标必须落在容器里、不进保留区、不出终端——穷举尺寸钉死。
    #[test]
    fn cursor_never_escapes_the_container_or_enters_the_reserved_strip() {
        for rows in 0u16..=40 {
            for reserved in 0u16..=4 {
                for input in 1u16..=6 {
                    let history = rows.saturating_sub(reserved).saturating_sub(input);
                    let reserved = reserved.min(rows);
                    for row in 0u16..=8 {
                        for col in [0u16, 3, 79, 200] {
                            let (x, y) =
                                cursor_position((80, rows), history, input, reserved, row, col);
                            assert!(x < 80, "列越界: {x}");
                            assert!(y < rows.max(1), "行越出终端: {y} >= {rows}");
                            if rows > reserved {
                                assert!(
                                    y < rows - reserved,
                                    "光标进了保留区: y={y} rows={rows} reserved={reserved}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// 容器内偏移正常时原样落位（容器顶 + 偏移）。
    #[test]
    fn cursor_maps_container_offsets_to_absolute_rows() {
        // 20 行终端，历史 12 + 输入 3 + 保留 5
        let pos = |row| cursor_position((60, 20), 12, 3, 5, row, 7);
        assert_eq!(pos(0), (7, 12));
        assert_eq!(pos(2), (7, 14));
        assert_eq!(pos(9), (7, 14), "超出的偏移该钳到容器最后一行");
    }

    /// 内宽扣掉边框，且永远 ≥1（窄终端不 panic、不出现 0 宽）。
    #[test]
    fn inner_width_always_leaves_room() {
        assert_eq!(inner_width(80), 76);
        assert_eq!(inner_width(4), 1);
        assert_eq!(inner_width(0), 1);
    }
}
