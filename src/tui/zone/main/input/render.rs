//! 输入区渲染 —— 状态栏当顶边，`| 正文 |` 当侧行，底边端走最后一行。
//!
//! 形状（用户定的规格）：
//! ```text
//! +--pi > [M] model > [D] path ... --+     <- 状态栏（1 行，即容器顶边）
//! | 第一行正文                       |     <- 只在正文多于一行时出现
//! | 第二行正文                       |
//! +-最后一行正文---------------------+     <- 底边；两端 +- / -+，中间只放内容
//! ```
//!
//! 规矩：
//! - 状态栏自己保证宽度恰好等于容器宽；这里 `debug_assert` 把它钉住，
//!   差一格下面每一行都会错位；
//! - 底边中间**只放内容**，不用横杠填满——用户就在这里打字；
//! - 换行按终端显示宽度算（`text` 负责）。
//!
//! 几何算术（边框几格、容器几行、光标跟随怎么滚）全在
//! [`crate::tui::zone::main::geometry`]，这里只消费。

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::tui::text;
use crate::tui::theme::Palette;
use crate::tui::zone::main::geometry;

/// 渲染结果：行列表 + 光标在容器内的位置。
#[derive(Debug, Clone)]
pub struct InputView {
    /// 容器全部行：第 0 行是状态栏。
    pub lines: Vec<Line<'static>>,
    /// 光标行（相对容器顶部，0 = 状态栏那一行）。
    pub cursor_row: usize,
    /// 光标终端绝对列（格）。
    pub cursor_col: usize,
}

/// 渲染输入区需要的一切。
pub struct Spec<'a> {
    /// 状态栏那一行，当容器顶边用。
    pub status_line: Line<'static>,
    /// 按 `inner_width` 换行后的输入文本。
    pub wrapped: &'a text::Wrapped,
    /// 视口第一行（换行行号）。
    pub starts: usize,
    /// 容器总行数（含状态栏与底边）。
    pub container_rows: usize,
    /// 光标所在的换行行号（绝对值，非视口相对）。
    pub cursor_row: usize,
    /// 光标在该换行行内的格偏移。
    pub cursor_col: usize,
    /// 终端宽度。
    pub term_w: u16,
}

/// 画输入区。
pub fn render(spec: &Spec<'_>, p: &Palette) -> InputView {
    let w = spec.term_w as usize;
    // 状态栏宽度**就是**容器宽度：差一格下面每一行都会撕裂。状态栏自己
    // 保证这点，这里是两边都不许赖账的闸。
    debug_assert_eq!(
        spec.status_line
            .spans
            .iter()
            .map(|s| text::display_width(&s.content))
            .sum::<usize>(),
        w,
        "状态栏必须恰好 {w} 格"
    );

    let wrapped = spec.wrapped;
    let n = wrapped.len();
    let starts = spec.starts.min(n.saturating_sub(1));
    // `| ... |` 行数 = 容器高 - 状态栏 - 底边
    let body_rows = spec.container_rows.saturating_sub(2);

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(spec.container_rows);
    lines.push(spec.status_line.clone());

    if n == 0 {
        // 空输入也要有一个底边可站：`+-` + 空格 + `-+`
        lines.push(bottom_line("", w, p));
        return InputView {
            lines,
            cursor_row: 1,
            cursor_col: 2,
        };
    }

    // 视口覆盖 `body_rows + 1` 行：前 `body_rows` 行走 `| ... |`，最后一行
    // 滑进底边。底边画的是**视口最后一行**而不是正文最后一行——否则往上
    // 滚之后底边还钉在正文末尾，画面就撕裂了。
    let visible = body_rows + 1;
    let bottom_row = (starts + visible - 1).min(n - 1);
    for i in starts..bottom_row {
        lines.push(bar_line(&wrapped.lines[i], w, p));
    }
    lines.push(bottom_line(&wrapped.lines[bottom_row], w, p));

    // 光标：状态栏占 1 行，再扣掉视口起点之前滚过去的行。
    let row = 1 + spec.cursor_row.saturating_sub(starts);
    // 正文行和底边都带 2 格前缀（`| ` / `+-`）。
    let col = 2 + spec.cursor_col;
    InputView {
        lines,
        cursor_row: row,
        cursor_col: col,
    }
}

/// 正文行：`| 内容 |`，左对齐，补齐到整宽。
fn bar_line(content: &str, w: usize, p: &Palette) -> Line<'static> {
    let inner = geometry::inner_width(w as u16);
    let (seg, seg_w) = text::take_width(content, inner);
    let pad = inner.saturating_sub(seg_w);
    Line::from(vec![
        p.accent_span("| "),
        Span::styled(seg, Style::new()),
        // 补白留空，别用横杠——这里是输入区，填空格才像可以打字
        Span::styled(" ".repeat(pad), Style::new()),
        p.accent_span(" |"),
    ])
}

/// 底边：`+-{内容}` + 补白 + `-+`。两端比状态栏各短一格横杠。
fn bottom_line(content: &str, w: usize, p: &Palette) -> Line<'static> {
    let inner = geometry::inner_width(w as u16);
    // 太宽就按显示宽度截断，边框不能破
    let (seg, seg_w) = text::take_width(content, inner);
    let pad = inner.saturating_sub(seg_w);
    Line::from(vec![
        p.accent_span("+-"),
        Span::styled(seg, Style::new()),
        Span::styled(" ".repeat(pad), Style::new()),
        p.accent_span("-+"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    fn width_of(l: &Line) -> usize {
        l.spans.iter().map(|s| s.content.width()).sum()
    }

    fn text_of(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.to_string()).collect()
    }

    fn status(w: u16) -> Line<'static> {
        Line::from("+".to_string() + &"-".repeat(w as usize - 2) + "+")
    }

    fn spec<'a>(
        wrapped: &'a text::Wrapped,
        container_rows: usize,
        starts: usize,
        cursor_row: usize,
        cursor_col: usize,
        term_w: u16,
    ) -> Spec<'a> {
        Spec {
            status_line: status(term_w),
            wrapped,
            starts,
            container_rows,
            cursor_row,
            cursor_col,
            term_w,
        }
    }

    /// 单行输入：只有状态栏 + 底边，且底边端的是正文。
    #[test]
    fn single_line_input_is_status_then_bottom() {
        let p = Palette::default();
        let wrapped = text::wrap("hi", 40);
        let v = render(&spec(&wrapped, 2, 0, 0, 2, 20), &p);
        assert_eq!(v.lines.len(), 2, "单行输入：状态栏 + 底边");
        assert!(text_of(&v.lines[1]).starts_with("+-hi"), "{:?}", text_of(&v.lines[1]));
        assert!(text_of(&v.lines[1]).ends_with("-+"));
    }

    /// 多行输入：中间的行走 `| ... |`，最后一行进底边。
    #[test]
    fn multiline_input_gets_bar_rows() {
        let p = Palette::default();
        let wrapped = text::wrap("aa\nbb\ncc", 40);
        let v = render(&spec(&wrapped, 4, 0, 2, 2, 20), &p);
        assert_eq!(v.lines.len(), 4, "状态栏 + 2 个 | 行 + 底边");
        assert!(text_of(&v.lines[1]).starts_with("| aa"));
        assert!(text_of(&v.lines[2]).starts_with("| bb"));
        assert!(text_of(&v.lines[3]).starts_with("+-cc"));
    }

    /// 每一行的显示宽度都必须**恰好**等于终端宽：多一格会把画面推歪。
    #[test]
    fn every_row_is_exactly_terminal_wide() {
        let p = Palette::default();
        for term_w in [8u16, 20, 40, 81] {
            let wrapped = text::wrap("一段中文 abcd efgh\n第二行", geometry::inner_width(term_w));
            let v = render(&spec(&wrapped, 5, 0, 1, 1, term_w), &p);
            for (i, l) in v.lines.iter().enumerate() {
                assert_eq!(
                    width_of(l),
                    term_w as usize,
                    "({term_w}) 第 {i} 行宽度不对: {:?}",
                    text_of(l)
                );
            }
        }
    }

    /// 视口滚动后底边跟着视口走，不能还钉在正文末尾。
    #[test]
    fn bottom_edge_follows_the_viewport_not_the_text_end() {
        let p = Palette::default();
        let wrapped = text::wrap("a\nb\nc\nd\ne", 40);
        // 容器 3 行 = 状态栏 + 1 个 | 行 + 底边 → 视口覆盖 2 行
        let v = render(&spec(&wrapped, 3, 0, 0, 0, 20), &p);
        assert!(text_of(&v.lines[1]).starts_with("| a"));
        assert!(text_of(&v.lines[2]).starts_with("+-b"), "底边该端第 2 行");
        // 滚到第 3 行起
        let v = render(&spec(&wrapped, 3, 2, 2, 0, 20), &p);
        assert!(text_of(&v.lines[1]).starts_with("| c"));
        assert!(text_of(&v.lines[2]).starts_with("+-d"), "滚动后底边没跟上");
    }

    /// 空输入也要有能站的底边，且光标落在底边行首。
    #[test]
    fn empty_input_still_has_a_bottom_edge() {
        let p = Palette::default();
        let wrapped = text::wrap("", 40);
        let v = render(&spec(&wrapped, 2, 0, 0, 0, 20), &p);
        assert_eq!(v.lines.len(), 2);
        assert_eq!(text_of(&v.lines[1]), format!("+-{}-+", " ".repeat(16)));
        assert_eq!((v.cursor_row, v.cursor_col), (1, 2));
    }

    /// 光标行随视口平移，且永远落在容器内。
    #[test]
    fn cursor_row_tracks_the_viewport() {
        let p = Palette::default();
        let wrapped = text::wrap("a\nb\nc\nd", 40);
        let v = render(&spec(&wrapped, 4, 2, 3, 0, 20), &p);
        assert_eq!(v.cursor_row, 1 + 3 - 2, "光标行没跟视口平移");
        assert!(v.cursor_row < v.lines.len(), "光标落到了容器外");
    }
}
