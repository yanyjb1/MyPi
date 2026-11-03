//! Rendering for `/resume`.
//!
//! One block per session: title, first-message preview and meta lines, with a
//! blank line between blocks. Selected rows are blue + bold, other text dim.

use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::zone::{ResumeZone, Row, Scope};
use crate::ui::tui::text::card::{card_edge, card_edge_labeled, card_row};
use crate::ui::tui::theme::fixed;

/// A span in one fixed color.
fn fg(text: impl Into<String>, color: ratatui::style::Color) -> Span<'static> {
    Span::styled(text.into(), Style::new().fg(color))
}

/// A bold span in one fixed color.
fn bold(text: impl Into<String>, color: ratatui::style::Color) -> Span<'static> {
    Span::styled(
        text.into(),
        Style::new().fg(color).add_modifier(Modifier::BOLD),
    )
}

/// Screen rows one session block takes: title + preview + meta + blank line.
///
/// The last block draws no blank line; the window math accounts for that.
fn block_height(has_preview: bool) -> usize {
    if has_preview { 4 } else { 3 }
}

pub(super) fn lines(z: &mut ResumeZone) -> Vec<Line<'static>> {
    let Some(size) = z.size() else {
        return Vec::new();
    };
    let width = size.cols as usize;
    // Frame: title + search + divider + list + divider + hint + bottom = 6 rows.
    let body_rows = (size.rows as usize).saturating_sub(6);
    let edge = Style::new().fg(fixed::DIM);
    let bg = Style::new();

    let mut out = Vec::with_capacity(size.rows as usize);

    // ---- Title bar: total count, scope ----
    let count = z.visible_len();
    let header = format!("会话 · {count} 个 · {}", z.scope_label());
    out.push(card_edge_labeled(
        edge,
        bg,
        width,
        Line::from(bold(header, fixed::TOOL_TITLE)),
    ));

    // ---- Search box ----
    let search = if z.filter().is_empty() {
        Line::from(vec![
            fg("> ", fixed::MD_HEADING),
            fg("输入关键字筛选（名字 / 首条消息 / 目录）", fixed::MUTED),
        ])
    } else {
        Line::from(vec![
            fg("> ", fixed::MD_HEADING),
            fg(z.filter().to_string(), fixed::SYSTEM),
        ])
    };
    out.push(card_row(search, edge, bg, width));
    out.push(card_edge(edge, bg, width));
    // Server-side refusals (delete rejected etc.) are drawn above the list, since
    // the main area is hidden while this selector is open.
    if z.failed() {
        out.push(card_row(
            Line::from(fg("删除被拒", fixed::ERR)),
            edge,
            bg,
            width,
        ));
    }

    // ---- List ----
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
        out.push(card_row(Line::from(fg(msg, fixed::MUTED)), edge, bg, width));
    } else {
        for i in top..(top + blocks).min(count) {
            let Some(row) = z.row_at(i) else {
                continue;
            };
            let selected = i == z.selected();
            let confirming = z.pending_delete() == Some(row.id);
            out.extend(row_lines(
                &row,
                selected,
                confirming,
                z.failed(),
                width,
                edge,
                bg,
            ));
            // Blank line between blocks; the last one is covered by body padding.
            if i + 1 < (top + blocks).min(count) {
                out.push(card_row(Line::default(), edge, bg, width));
            }
        }
    }

    // Fill the remaining height so a short list leaves no unpainted gap.
    while out.len() < body_rows + 3 {
        out.push(card_row(Line::default(), edge, bg, width));
    }

    // ---- Footer: key hints ----
    out.push(card_edge(edge, bg, width));
    out.push(card_row(footer(), edge, bg, width));
    out.push(card_edge(edge, bg, width));
    out
}

/// The three lines of one session block.
fn row_lines(
    row: &Row,
    selected: bool,
    confirming: bool,
    refused: bool,
    width: usize,
    edge: Style,
    bg: Style,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();

    // Line 1: cursor + title; selected rows are blue + bold.
    // The cursor is the only position indicator, and its blank placeholder keeps
    // every title starting at the same column.
    let cursor = if selected { "> " } else { "  " };
    let cursor_color = if selected {
        fixed::MD_HEADING
    } else {
        fixed::MUTED
    };
    // Titles are always bold; only the color marks the selection.
    let title_color = if selected {
        fixed::MD_HEADING
    } else {
        fixed::BODY
    };
    let mut spans = vec![fg(cursor, cursor_color)];
    let title = super::zone::title(row);
    spans.push(bold(title, title_color));
    // Second step of the two-step delete: Delete raises the prompt, Enter commits.
    if confirming {
        let (label, color) = if refused {
            (" - fail", fixed::WARN)
        } else {
            (" - delete?", fixed::ERR)
        };
        spans.push(bold(label, color));
    }
    out.push(card_row(Line::from(spans), edge, bg, width));

    // Line 2: first-message preview, only for sessions with an explicit name.
    if let Some(preview) = super::zone::preview(row) {
        out.push(card_row(
            Line::from(fg(format!("  {preview}"), fixed::DIM)),
            edge,
            bg,
            width,
        ));
    }

    // Line 3: age, size, directory, each segment colored separately.
    let mut meta: Vec<Span<'static>> = vec![fg("  ", fixed::MUTED)];
    let dot = || fg(" · ", fixed::MUTED);
    let stamp = row.last_ts.as_deref().unwrap_or(&row.started_at);
    meta.push(fg(age_text(stamp), fixed::DIM));
    meta.push(dot());
    if let Some(bytes) = row.bytes {
        meta.push(fg(human_bytes(bytes), fixed::DIM));
    }
    if row.memos > 0 {
        meta.push(dot());
        meta.push(fg(format!("memo {}", row.memos), fixed::DIM));
    }
    if let Some(cwd) = row.cwd.as_deref() {
        meta.push(dot());
        meta.push(fg(shorten(cwd), fixed::DIM));
    }
    out.push(card_row(Line::from(meta), edge, bg, width));

    out
}

/// Footer key hints, drawn on the bottom row.
fn footer() -> Line<'static> {
    Line::from(vec![
        fg("Enter", fixed::SYSTEM),
        fg(" 附着 · ", fixed::MUTED),
        fg("Esc", fixed::SYSTEM),
        fg(" 返回 · ", fixed::MUTED),
        fg("↑↓", fixed::SYSTEM),
        fg(" 选择 · ", fixed::MUTED),
        fg("Tab", fixed::SYSTEM),
        fg(" 范围 · ", fixed::MUTED),
        fg("Ctrl+U", fixed::SYSTEM),
        fg(" 清筛选", fixed::MUTED),
    ])
}

/// List window: the start index `top` and how many blocks fit.
///
/// The budget is in physical rows (a block is 3 or 4 rows), so a short last block
/// is never pushed off screen.
fn window(z: &ResumeZone, budget: usize) -> (usize, usize) {
    let count = z.visible_len();
    if count == 0 || budget == 0 {
        return (0, 0);
    }
    let mut top = z.scroll().min(count - 1);
    // Selected row is above the window: follow it.
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
        // Selected block is outside the window: move it down one and retry.
        if z.selected() >= top + seen && seen > 0 {
            top += 1;
            continue;
        }
        return (seen.max(1), top);
    }
}

fn has_preview(z: &ResumeZone, i: usize) -> bool {
    z.row_at(i)
        .map(|r| super::zone::preview(r).is_some())
        .unwrap_or(false)
}

fn age_text(stamp: &str) -> String {
    let Some(then) = stamp_epoch(stamp) else {
        return stamp.to_string();
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(then);
    human_age(now.saturating_sub(then).max(0) as u64)
}

/// `started_at` is local wall time (`YYYY-MM-DD HH:MM:SS`); subtract the local offset to
/// compare it with the epoch clock.
fn stamp_epoch(stamp: &str) -> Option<i64> {
    let (date, time) = stamp.split_once(' ')?;
    let mut date = date.split('-');
    let y = date.next()?.parse::<i64>().ok()?;
    let m = date.next()?.parse::<i64>().ok()?;
    let d = date.next()?.parse::<i64>().ok()?;
    let mut time = time.split(':');
    let hh = time.next()?.parse::<i64>().ok()?;
    let mm = time.next()?.parse::<i64>().ok()?;
    let ss = time.next()?.parse::<i64>().ok()?;
    Some(days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss - local_offset())
}

/// Howard Hinnant's civil-date algorithm: days since 1970-01-01.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[repr(C)]
struct Tm {
    tm_sec: i32,
    tm_min: i32,
    tm_hour: i32,
    tm_mday: i32,
    tm_mon: i32,
    tm_year: i32,
    tm_wday: i32,
    tm_yday: i32,
    tm_isdst: i32,
    tm_gmtoff: i64,
    tm_zone: *const i8,
}

unsafe extern "C" {
    fn localtime_r(time: *const i64, out: *mut Tm) -> *mut Tm;
}

fn local_offset() -> i64 {
    static OFFSET: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *OFFSET.get_or_init(|| {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let mut tm: Tm = unsafe { std::mem::zeroed() };
        unsafe {
            localtime_r(&now, &mut tm);
        }
        tm.tm_gmtoff
    })
}

/// Relative age in Chinese: just now / N minutes / N hours / N days.
fn human_age(secs: u64) -> String {
    match secs {
        0..=59 => "刚刚".to_string(),
        60..=3599 => format!("{} 分钟前", secs / 60),
        3600..=86_399 => format!("{} 小时前", secs / 3600),
        86_400..=2_591_999 => format!("{} 天前", secs / 86_400),
        _ => format!("{} 天前", secs / 86_400),
    }
}

/// Byte size; decimals only where they carry information.
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

/// Path shortening: at most the last two segments, no leading slash,
/// prefixed with an ellipsis marker when segments were dropped.
fn shorten(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() <= 2 {
        return parts.join("/");
    }
    format!("…/{}", parts[parts.len() - 2..].join("/"))
}
