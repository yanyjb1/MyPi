// Rendering for the session picker: the page chassis, filled with one block per session.

use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::json;

use super::zone::{ResumeZone, Row, Scope};
use crate::ui::tui::lang::Lang;
use crate::ui::tui::text::bytes as byte_size;
use crate::ui::tui::theme::fixed;
use crate::ui::tui::zone::page::{Filter, Hint, Panel};

// One sentence of this language, its `{name}` holes filled.
fn text(lang: Lang, key: &str, args: serde_json::Value) -> String {
    crate::i18n::render(lang.code(), key, &args)
}

// A span in one fixed color.
fn fg(text: impl Into<String>, color: ratatui::style::Color) -> Span<'static> {
    Span::styled(text.into(), Style::new().fg(color))
}

// A bold span in one fixed color.
fn bold(text: impl Into<String>, color: ratatui::style::Color) -> Span<'static> {
    Span::styled(
        text.into(),
        Style::new().fg(color).add_modifier(Modifier::BOLD),
    )
}

// Screen rows one session block takes: title + preview + meta + a gap after it.
fn block_height(has_preview: bool) -> usize {
    if has_preview { 4 } else { 3 }
}

pub(super) fn lines(z: &mut ResumeZone) -> Vec<Line<'static>> {
    let Some(size) = z.size() else {
        return Vec::new();
    };
    let lang = z.lang();
    let scope = match z.scope() {
        Scope::Current => text(lang, "resume_scope_current", json!({})),
        Scope::All => text(lang, "resume_scope_all", json!({})),
    };
    let header = text(
        lang,
        "resume_title",
        json!({"count": z.visible_len(), "scope": scope}),
    );

    let mut extra = Vec::new();
    if z.failed() {
        extra.push(Line::from(fg(
            text(lang, "resume_delete_failed", json!({})),
            fixed::ERR,
        )));
    }
    for word in z.opening() {
        extra.push(Line::from(fg(word.clone(), fixed::WARN)));
    }

    let panel = Panel {
        size,
        title: header,
        filter: Some(Filter {
            text: z.filter().to_string(),
            placeholder: text(lang, "resume_filter_hint", json!({})),
            caption: text(lang, "resume_filter_caption", json!({})),
        }),
        extra,
        empty: empty(z, lang),
        hints: hints(lang),
        accent: z.accent(),
    };

    let rows = panel.body_rows();
    let (blocks, top) = window(z, rows);
    z.set_scroll(top);

    let count = z.visible_len();
    let mut body = Vec::new();
    for i in top..(top + blocks).min(count) {
        let Some(row) = z.row_at(i) else {
            continue;
        };
        let selected = i == z.selected();
        let confirming = z.pending_delete() == Some(row.id);
        body.extend(row_lines(&row, selected, confirming, z.failed(), lang));
        // Blank line between blocks; the last one is covered by the body padding.
        if i + 1 < (top + blocks).min(count) {
            body.push(Line::default());
        }
    }
    panel.draw(body)
}

// What the list says when it has nothing to show.
fn empty(z: &ResumeZone, lang: Lang) -> Option<Line<'static>> {
    if let Some(error) = z.error() {
        let word = text(lang, "resume_read_failed", json!({"detail": error}));
        return Some(Line::from(fg(word, fixed::ERR)));
    }
    if z.visible_len() > 0 {
        return None;
    }
    let key = if !z.loaded() {
        "resume_empty_loading"
    } else if z.scope() == Scope::Current {
        "resume_empty_current"
    } else if z.filter().is_empty() {
        "resume_empty_all"
    } else {
        "resume_empty_filtered"
    };
    Some(Line::from(fg(text(lang, key, json!({})), fixed::MUTED)))
}

// The three lines of one session block.
fn row_lines(
    row: &Row,
    selected: bool,
    confirming: bool,
    refused: bool,
    lang: Lang,
) -> Vec<Line<'static>> {
    let mut out = Vec::new();

    // The cursor is the only position indicator, and its blank placeholder keeps every title aligned.
    let cursor = if selected { "> " } else { "  " };
    let cursor_color = if selected {
        fixed::MD_HEADING
    } else {
        fixed::MUTED
    };
    let title_color = if selected {
        fixed::MD_HEADING
    } else {
        fixed::BODY
    };
    let mut spans = vec![fg(cursor, cursor_color)];
    spans.push(bold(super::zone::title(row), title_color));
    // Second step of the two-step delete: Delete raises the prompt, Enter commits.
    if confirming {
        let key = if refused {
            "resume_confirm_failed"
        } else {
            "resume_confirm_delete"
        };
        let color = if refused { fixed::WARN } else { fixed::ERR };
        spans.push(bold(text(lang, key, json!({})), color));
    }
    out.push(Line::from(spans));

    if let Some(preview) = super::zone::preview(row) {
        out.push(Line::from(fg(format!("  {preview}"), fixed::DIM)));
    }

    // Age, size, memos and directory, each segment colored separately.
    let mut meta: Vec<Span<'static>> = vec![fg("  ", fixed::MUTED)];
    let dot = || fg(" · ", fixed::MUTED);
    let stamp = row.last_ts.as_deref().unwrap_or(&row.started_at);
    meta.push(fg(age_text(stamp, lang), fixed::DIM));
    meta.push(dot());
    if let Some(bytes) = row.bytes {
        meta.push(fg(byte_size(bytes.max(0) as u64), fixed::DIM));
    }
    if row.memos > 0 {
        meta.push(dot());
        let word = text(lang, "resume_memos", json!({"count": row.memos}));
        meta.push(fg(word, fixed::DIM));
    }
    if let Some(cwd) = row.cwd.as_deref() {
        meta.push(dot());
        meta.push(fg(shorten(cwd), fixed::DIM));
    }
    out.push(Line::from(meta));
    out
}

// Every key the picker answers to. The order is what survives a short frame: the three a reader
// needs at any width lead, so a narrow terminal drops the tail instead.
fn hints(lang: Lang) -> Vec<Hint> {
    let hint = |keys: &'static str, key: &str| Hint::new(keys, text(lang, key, json!({})));
    vec![
        hint("↑↓", "hint_select"),
        hint("Enter", "hint_attach"),
        hint("Esc", "hint_back"),
        hint("PgUp/PgDn", "hint_page"),
        hint("Home/End", "hint_ends"),
        hint("Delete", "hint_delete"),
        hint("Tab", "hint_scope"),
        hint("Ctrl+U", "hint_clear"),
    ]
}

// List window: the start index `top` and how many blocks fit.
// The budget is in physical rows (a block is 3 or 4 rows), so a short last block is never pushed off screen.
fn window(z: &ResumeZone, budget: usize) -> (usize, usize) {
    let count = z.visible_len();
    if count == 0 || budget == 0 {
        return (0, 0);
    }
    let mut top = z.scroll().min(count - 1);
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

fn age_text(stamp: &str, lang: Lang) -> String {
    let Some(then) = stamp_epoch(stamp) else {
        return stamp.to_string();
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(then);
    human_age(now.saturating_sub(then).max(0) as u64, lang)
}

// `started_at` is local wall time (`YYYY-MM-DD HH:MM:SS`); the local offset is subtracted so it compares with the epoch clock.
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

// Howard Hinnant's civil-date algorithm: days since 1970-01-01.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

// `UTC+8` / `GMT-5` / `UTC+8:30` → seconds east of UTC; anything else is UTC.
fn parse_offset(name: &str) -> i64 {
    let name = name.trim();
    let Some(rest) = name
        .strip_prefix("UTC")
        .or_else(|| name.strip_prefix("GMT"))
    else {
        return 0;
    };
    let Some((sign, digits)) = rest.split_at_checked(1) else {
        return 0;
    };
    let mult = match sign {
        "+" => 1,
        "-" => -1,
        _ => return 0,
    };
    let (h, m) = digits.split_once(':').unwrap_or((digits, "0"));
    let (Ok(h), Ok(m)) = (h.parse::<i64>(), m.parse::<i64>()) else {
        return 0;
    };
    if h > 24 || m >= 60 {
        return 0;
    }
    mult * (h * 3600 + m * 60)
}

// The offset the server answered; UTC until that answer lands.
static OFFSET: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

pub(super) fn set_local_offset(name: &str) {
    OFFSET.store(parse_offset(name), std::sync::atomic::Ordering::Relaxed);
}

fn local_offset() -> i64 {
    OFFSET.load(std::sync::atomic::Ordering::Relaxed)
}

// Relative age in the page's language.
fn human_age(secs: u64, lang: Lang) -> String {
    let (key, n) = match secs {
        0..=59 => return text(lang, "resume_age_now", json!({})),
        60..=3599 => ("resume_age_minutes", secs / 60),
        3_600..=86_399 => ("resume_age_hours", secs / 3_600),
        _ => ("resume_age_days", secs / 86_400),
    };
    text(lang, key, json!({"n": n}))
}

// Path shortening: at most the last two segments, no leading slash, an ellipsis when segments were dropped.
fn shorten(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.len() <= 2 {
        return parts.join("/");
    }
    format!("…/{}", parts[parts.len() - 2..].join("/"))
}
