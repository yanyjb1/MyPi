//! `/resume` 的画法。
//!
//! 布局照 omp 的 `session-selector.ts`：一条会话一块，块内三行
//! （标题 / 首条消息预览 / 元信息），块间空一行；选中行 accent + 粗体，
//! 其余文字 dim。框线复用卡片原语，所以整页和工具卡片是一套视觉。

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::{ResumeZone, Scope};
use crate::tui::zone::main::history::render::cards::{card_edge, card_edge_labeled, card_row};
use crate::tui::zone::main::history::render::theme::{HistoryTheme, Token};

/// 一条会话在屏幕上的块高（标题 + 预览 + 元信息 + 空行）。
///
/// 列表里最下面那条不画空行（footer 自带间隔），窗口计算按 py 处理。
fn block_height(has_preview: bool) -> usize {
    if has_preview { 4 } else { 3 }
}

pub(super) fn lines(z: &mut ResumeZone) -> Vec<Line<'static>> {
    let t = HistoryTheme::resolve();
    let Some(size) = z.size() else {
        return Vec::new();
    };
    let width = size.cols as usize;
    // 框架：标题 + 搜索 + 分隔 + 列表 + 分隔 + 提示 + 底边 = 6 行。
    let body_rows = (size.rows as usize).saturating_sub(6);
    let edge = t.fg_style(Token::Dim);
    let bg = Style::new();

    let mut out = Vec::with_capacity(size.rows as usize);

    // ---- 标题栏：一共几条 · 作用域 ----
    let count = z.visible_len();
    let header = format!(
        "会话 · {count} 个 · {}",
        z.scope_label()
    );
    out.push(card_edge_labeled(
        edge,
        bg,
        width,
        Line::from(t.fg_mod(header, Token::ToolTitle, Modifier::BOLD)),
    ));

    // ---- 搜索框 ----
    let search = if z.filter().is_empty() {
        Line::from(vec![
            t.fg("> ", Token::ToolEdgePending),
            t.fg("输入关键字筛选（名字 / 首条消息 / 目录）", Token::Muted),
        ])
    } else {
        Line::from(vec![
            t.fg("> ", Token::ToolEdgePending),
            t.fg(z.filter().to_string(), Token::SystemText),
        ])
    };
    out.push(card_row(search, edge, bg, width));
    out.push(card_edge(edge, bg, width));
    // 服务端的拒绝（删除被拒之类）就画在列表上方：选择器占着整屏时，
    // 主区的错误注记用户看不见。
    if let Some(err) = z.error() {
        out.push(card_row(
            Line::from(t.fg(err.to_string(), Token::Err)),
            edge,
            bg,
            width,
        ));
    }

    // ---- 列表 ----
    let (blocks, top) = window(z, body_rows);
    z.set_scroll(top);

    if count == 0 {
        let msg = if !z.loaded() {
            "正在读取…"
        } else if z.scope() == Scope::Current {
            "这个目录下还没有会话（Tab 看全部）"
        } else if z.filter().is_empty() {
            "还没有任何会话"
        } else {
            "没有匹配的会话（退格改筛选）"
        };
        out.push(card_row(Line::from(t.fg(msg, Token::Muted)), edge, bg, width));
    } else {
        for i in top..(top + blocks).min(count) {
            let Some(row) = z.row_at(i) else { continue };
            let selected = i == z.selected();
            let confirming = z.pending_delete() == Some(row.id());
            out.extend(row_lines(&t, &row, selected, confirming, width, edge, bg));
            // 块间空行；最下面那条留白由 body 的填充补齐。
            if i + 1 < (top + blocks).min(count) {
                out.push(card_row(Line::default(), edge, bg, width));
            }
        }
    }

    // 填满剩余高度：列表短的时候下面不该出现未上色的空隙。
    while out.len() < body_rows + 3 {
        out.push(card_row(Line::default(), edge, bg, width));
    }

    // ---- 页脚：按键提示 ----
    out.push(card_edge(edge, bg, width));
    out.push(card_row(footer(&t), edge, bg, width));
    out.push(card_edge(edge, bg, width));
    out
}

/// 一条会话的三行。
fn row_lines(
    t: &HistoryTheme,
    row: &super::RowViewRef<'_>,
    selected: bool,
    confirming: bool,
    width: usize,
    edge: Style,
    bg: Style,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();

    // 第一行：光标 + 标题。选中行 accent + 粗体（照 omp：cursor 是唯一
    // 的位置指示，其余行用等宽空白占位，所以整列文字对齐）。
    // 光标是唯一的位置指示；未选中行用等宽空白占位，整列文字因此对齐。
    let cursor = if selected { "> " } else { "  " };
    let cursor_tok = if selected { Token::ToolEdgePending } else { Token::Muted };
    // 标题一律**加粗**：名字（不管是用户起的还是合成出来的）是第一眼要
    // 认出来的东西。选中行唱 accent，其余用正文色（`text`，即白）——
    // 灰色会让标题和下面的元信息糊成一片。
    let title_tok = if selected { Token::ToolEdgePending } else { Token::AssistantText };
    let mut spans = vec![t.fg(cursor, cursor_tok)];
    let title = row.title();
    spans.push(t.fg_mod(title, title_tok, Modifier::BOLD));
    // 两段式删除的第二步提示：Delete 亮出来，Enter 才真删——红色加粗，
    // 免得用户以为是随手多出来的一行字。
    if confirming {
        spans.push(t.fg_mod(" - delete?", Token::Err, Modifier::BOLD));
    }
    out.push(card_row(Line::from(spans), edge, bg, width));

    // 第二行：首条消息预览（只有显式命名的会话才画，见 `Row::preview`）。
    if let Some(preview) = row.preview() {
        out.push(card_row(
            Line::from(t.fg(format!("  {preview}"), Token::Dim)),
            edge,
            bg,
            width,
        ));
    }

    // 第三行：多久以前 · 多大 · 在哪个目录。每段单独上色（omp 同款理由：
    // 整行一起包色会把状态段的颜色吃掉）。
    let mut meta: Vec<Span<'static>> = vec![t.fg("  ", Token::Muted)];
    let dot = || t.fg(" · ", Token::Muted);
    match row.age() {
        Some(age) => meta.push(t.fg(human_age(age), Token::Dim)),
        // 戳坏了：画原文，别画一个错的年龄。
        None => meta.push(t.fg(row.stamp().to_string(), Token::Dim)),
    }
    meta.push(dot());
    meta.push(t.fg(human_bytes(row.bytes()), Token::Dim));
    if let Some(cwd) = row.cwd() {
        meta.push(dot());
        meta.push(t.fg(shorten(cwd), Token::Dim));
    }
    out.push(card_row(Line::from(meta), edge, bg, width));

    out
}

/// 页脚提示。o mp 的 overlay 把提示放在底部同一行，我们照做。
fn footer(t: &HistoryTheme) -> Line<'static> {
    Line::from(vec![
        t.fg("Enter", Token::SystemText),
        t.fg(" 附着 · ", Token::Muted),
        t.fg("Esc", Token::SystemText),
        t.fg(" 返回 · ", Token::Muted),
        t.fg("↑↓", Token::SystemText),
        t.fg(" 选择 · ", Token::Muted),
        t.fg("Tab", Token::SystemText),
        t.fg(" 范围 · ", Token::Muted),
        t.fg("Ctrl+U", Token::SystemText),
        t.fg(" 清筛选", Token::Muted),
    ])
}

/// 算列表窗口：起点 `top` 与能放下几个块。
///
/// 窗口按**物理行**预算滚动（块高 3 或 4 行），所以短会话不会把最下面
/// 那条挤到屏幕外——按条数算窗口会留一堆看不了的空白。
fn window(z: &ResumeZone, budget: usize) -> (usize, usize) {
    let count = z.visible_len();
    if count == 0 || budget == 0 {
        return (0, 0);
    }
    let mut top = z.scroll().min(count - 1);
    // 选中行在窗口上面：窗口跟上去。
    if z.selected() < top {
        top = z.selected();
    }
    loop {
        let mut used = 0usize;
        let mut seen = 0usize;
        for i in top..count {
            let h = block_height(has_preview(z, i)) + usize::from(i + 1 < count);
            if used + h > budget && seen > 0 {
                break;
            }
            used += h;
            seen += 1;
        }
        // 选中的块没进来：窗口往下挪一格再试。
        if z.selected() >= top + seen && seen > 0 {
            top += 1;
            continue;
        }
        return (seen.max(1), top);
    }
}

fn has_preview(z: &ResumeZone, i: usize) -> bool {
    z.row_at(i).map(|r| r.preview().is_some()).unwrap_or(false)
}

/// 相对时间。中文用「刚刚 / N 分钟前 / N 小时前 / N 天前」，再老就没有
/// 相对的意义了，画原戳的日期部分。
fn human_age(secs: u64) -> String {
    match secs {
        0..=59 => "刚刚".to_string(),
        60..=3599 => format!("{} 分钟前", secs / 60),
        3600..=86_399 => format!("{} 小时前", secs / 3600),
        86_400..=2_591_999 => format!("{} 天前", secs / 86_400),
        // 30 天以上：相对时间反而要靠心算。
        _ => format!("{} 天前", secs / 86_400),
    }
}

/// 体积。跟 `format_duration` 一个思路：只在有意义时才带小数。
fn human_bytes(bytes: i64) -> String {
    let b = bytes.max(0) as f64;
    if b < 1024.0 {
        return format!("{} B", bytes.max(0));
    }
    let kb = b / 1024.0;
    if kb < 1024.0 {
        return format!("{:.1} KB", kb);
    }
    format!("{:.1} MB", kb / 1024.0)
}

/// 目录压缩显示：`/home/me/projects/a` → `~/projects/a` 太长就取尾两段。
///
/// 与状态栏的 `tmp/mypi-fake` 风格一致：去掉前导斜杠，只留能认出来的尾巴。
fn shorten(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() <= 2 {
        return parts.join("/");
    }
    format!("…/{}", parts[parts.len() - 2..].join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 整页的画法：标题栏、两条会话各自的块、页脚，全部塞得进终端宽度。
    ///
    /// 「塞不进」是这类页面的典型故障：页脚被裁掉半截（CJK 是两格宽，
    /// 数错就少画一个字），或者正文行超出边框把右边框推走。
    #[test]
    fn the_page_fits_the_terminal_and_says_what_it_is() {
        use crate::tui::zone::resume::ResumeZone;
        use crate::tui::zone::{TermSize, Zone as _};
        let mut z = ResumeZone::default();
        z.attach(TermSize {
            cols: 110,
            rows: 24,
        });
        z.begin(std::path::PathBuf::from("/proj/a"));
        z.on_sessions(vec![
            crate::server::wire::SessionInfo {
                id: 2,
                name: Some("给会话起的名字".into()),
                started_at: "2026-09-22 14:30:05".into(),
                cwd: Some("/tmp/mypi-fake/work".into()),
                first_message: Some("首条消息\n第二行".into()),
                bytes: 2048,
            },
            crate::server::wire::SessionInfo {
                id: 1,
                name: None,
                started_at: "2026-09-22 13:00:00".into(),
                cwd: Some("/proj/a".into()),
                first_message: Some("没名字的会话".into()),
                bytes: 0,
            },
        ]);

        let lines = crate::tui::zone::Zone::render(&mut z);
        let text: Vec<String> = lines.iter().map(|l| l.to_string()).collect();
        let all = text.join("\n");

        assert!(text[0].contains("会话 · 2 个 · 当前目录"), "标题栏：{}", text[0]);
        assert!(all.contains("给会话起的名字"), "有名字的会话画名字");
        assert!(all.contains("没名字的会话"), "没名字的会话画首条消息");
        assert!(all.contains("首条消息"), "有名字的会话补一行预览");
        assert!(all.contains("2.0 KB"), "体积");
        assert!(all.contains("…/mypi-fake/work"), "目录尾巴");
        let footer = text.iter().find(|l| l.contains("Enter")).expect("页脚");
        assert!(footer.contains("选择"), "页脚被裁掉了一个字：{footer}");
        assert!(footer.contains("清筛选"), "页脚被裁：{footer}");
        for (i, l) in lines.iter().enumerate() {
            assert!(
                l.width() <= 110,
                "第 {i} 行 {} 格，超出 110 的终端宽度：{:?}",
                l.width(),
                text[i]
            );
        }
    }

    #[test]
    fn ages_read_like_a_person_wrote_them() {
        assert_eq!(human_age(0), "刚刚");
        assert_eq!(human_age(59), "刚刚");
        assert_eq!(human_age(60), "1 分钟前");
        assert_eq!(human_age(3 * 3600 + 5), "3 小时前");
        assert_eq!(human_age(2 * 86_400), "2 天前");
    }

    #[test]
    fn sizes_stay_readable_across_the_range() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2.0 KB");
        assert_eq!(human_bytes(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(human_bytes(-1), "0 B", "负数不该画成负数");
    }

    #[test]
    fn long_directories_keep_their_tail() {
        assert_eq!(shorten("/tmp/a"), "tmp/a");
        assert_eq!(shorten("/home/me/projects/a"), "…/projects/a");
    }
}
