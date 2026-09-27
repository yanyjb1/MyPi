//! Completion popup — the candidate list shown above the input container.
//!
//! Why above the container instead of overlaying the input:
//! an overlay would hide the text being typed — exactly what the user reads to pick a candidate.
//!
//! Alignment contract (the whole point of this module's layout):
//! the **first character of every candidate name sits at exactly the cursor's
//! column**. Command rows park their `>` marker in the two columns *left* of that
//! column, so the marker never pushes the name rightward.

use ratatui::text::Line;

use super::engine::{CandidateKind, CompletionPopup, VisibleRow};
use crate::tui::theme::Palette;

// The two columns a command row spends on its selection marker, immediately
// left of the anchor column.
const MARKER_W: usize = 2;

// The popup's rendering result.
pub struct PopupView {
    // Rows, top to bottom (item 0 renders at the very top).
    //
    // Already carry their own leading indent: each row aligns itself to
    // `anchor_col`, so the caller must NOT add any indent of its own.
    //
    // Screen order, never reversed: ↑ moves up,
    // matching what the user sees — no "pressed up, jumped to the top" confusion.
    pub lines: Vec<Line<'static>>,
    // 弹窗锚定的终端绝对列（= 光标列）。命令行的名字就落在这一列上。
    pub anchor_col: usize,
}

// Render the popup. Empty content returns nothing.
//
// `avail` is the popup's row capacity (given by the caller from the space above the container).
pub fn render(
    popup: &CompletionPopup,
    term_w: u16,
    avail: usize,
    anchor_col: usize,
    p: &Palette,
) -> PopupView {
    let visible = popup.visible(avail);
    if visible.is_empty() {
        return PopupView {
            lines: Vec::new(),
            anchor_col: 0,
        };
    }

    // Locked height: pad with blank rows when candidates shrink; the reserved area stays put (no jitter)
    let target = popup.locked_height().unwrap_or(visible.len());

    let inner = crate::tui::zone::main::geometry::inner_width(term_w);

    // Content width feeds the overflow rule: a row that would run past the inner
    // edge pulls the whole popup left so its right edge lands on that edge.
    let popup_w = visible
        .iter()
        .map(|r| row_content_width(r.name, r.detail))
        .max()
        .unwrap_or(0);
    let anchor_col = if anchor_col + popup_w > inner {
        inner.saturating_sub(popup_w)
    } else {
        anchor_col
    };

    // Descriptions are one column across every row: pad names to the widest one so
    // the first character of every description lands on the same x.
    let name_col = visible
        .iter()
        .filter(|r| r.kind == CandidateKind::Command && !r.detail.is_empty())
        .map(|r| crate::tui::text::display_width(r.name))
        .max()
        .unwrap_or(0);

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(visible.len());

    for row in visible {
        let VisibleRow {
            kind,
            name,
            hits,
            detail,
            selected,
            is_dir,
        } = row;
        // Every row hangs off the same anchor; `>` and its trailing space are paid
        // for out of the two columns to the left of it.
        let mut spans: Vec<ratatui::text::Span<'static>> = Vec::new();
        spans.push(p.plain(" ".repeat(anchor_col.saturating_sub(MARKER_W))));
        spans.push(p.plain(if selected { "> " } else { "  " }));

        // The name is drawn as a run of spans: matched characters accent, the rest
        // in normal foreground, so the list shows why each candidate is in it.
        spans.extend(name_spans(kind, name, hits, is_dir, p));

        if !detail.is_empty() {
            let pad = name_col.saturating_sub(crate::tui::text::display_width(name));
            spans.push(p.plain(format!("{}  ", " ".repeat(pad))));
            spans.push(p.muted(detail.to_string()));
        }

        lines.push(Line::from(spans));
    }

    // Pad blank placeholder rows up to the locked height
    while lines.len() < target {
        lines.push(Line::from(""));
    }

    PopupView {
        lines,
        anchor_col,
    }
}

// Split a candidate name into accented and plain runs.
//
// Two rules, one per kind:
// - `Command` (`/model`): the **whole** name is accent, always. The accent's job
//   there is to separate the command from its description, and there are only a
//   handful of commands — highlighting the typed prefix would just look ragged.
// - `Plain` (paths, profile names, model ids): only the matched characters are
//   accent. Here the list is long and unsorted-by-relevance, so showing which
//   characters matched is what tells the user why a row is in it.
//
// `hits` are char indices into `name`. A directory's trailing `/` is never
// accented under the `Plain` rule: the matcher never looked at it, so accenting
// it would claim a match that did not happen.
fn name_spans(
    kind: CandidateKind,
    name: &str,
    hits: &[usize],
    is_dir: bool,
    p: &Palette,
) -> Vec<ratatui::text::Span<'static>> {
    if kind == CandidateKind::Command {
        return vec![p.accent_span(name.to_string())];
    }
    let chars: Vec<char> = name.chars().collect();
    // The trailing directory slash is decoration, not part of the name proper:
    // the matcher never looked at it, so accenting it would claim a match that
    // never happened.
    let name_len = if is_dir {
        chars.len().saturating_sub(1)
    } else {
        chars.len()
    };
    if hits.is_empty() {
        return vec![p.plain(name.to_string())];
    }
    let hit = |i: usize| hits.binary_search(&i).is_ok();
    let mut spans: Vec<ratatui::text::Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_hit = hit(0);
    for (i, &c) in chars.iter().enumerate() {
        let this_hit = i < name_len && hit(i);
        if this_hit != run_hit {
            if !run.is_empty() {
                spans.push(if run_hit {
                    p.accent_span(std::mem::take(&mut run))
                } else {
                    p.plain(std::mem::take(&mut run))
                });
            }
            run_hit = this_hit;
        }
        run.push(c);
    }
    if !run.is_empty() {
        spans.push(if run_hit {
            p.accent_span(run)
        } else {
            p.plain(run)
        });
    }
    spans
}

// Width of a row's content in display columns, ignoring the leading indent
// (every row is indented to the same anchor, so the indent cancels out when
// comparing rows). Used only for the overflow decision.
fn row_content_width(name: &str, detail: &str) -> usize {
    let name_w = crate::tui::text::display_width(name);
    if detail.is_empty() {
        return name_w + MARKER_W;
    }
    name_w + 2 + crate::tui::text::display_width(detail) + MARKER_W
}

#[cfg(test)]
mod tests {
    use super::super::engine::{CandidateKind, Completion, CompletionPopup};
    use super::*;

    fn cand(name: &str, is_dir: bool) -> Completion {
        Completion {
            kind: CandidateKind::Plain,
            hits: Vec::new(),
            name: name.to_string(),
            detail: String::new(),
            is_dir,
            insert: name.to_string(),
        }
    }

    /// 路径候选：Plain 类，hits 显式给出。
    fn path_cand(name: &str, hits: &[usize]) -> Completion {
        Completion {
            kind: CandidateKind::Plain,
            name: name.to_string(),
            hits: hits.to_vec(),
            detail: String::new(),
            is_dir: name.ends_with('/'),
            insert: name.to_string(),
        }
    }

    fn cmd(name: &str, detail: &str) -> Completion {
        Completion {
            kind: CandidateKind::Command,
            hits: Vec::new(),
            name: name.to_string(),
            detail: detail.to_string(),
            is_dir: false,
            insert: format!("{name} "),
        }
    }

    fn text_of(l: &Line) -> String {
        l.spans.iter().map(|s| s.content.to_string()).collect()
    }

    /// Column where the *name* begins.
    ///
    /// Measured on the rendered text: skip the leading pad, then the marker cell
    /// (`> ` or `  `), and report where the first name character lands.
    fn name_col_of(l: &Line) -> usize {
        let text = text_of(l);
        let chars: Vec<char> = text.chars().collect();
        // Leading pad.
        let mut i = 0;
        while i < chars.len() && chars[i] == ' ' {
            i += 1;
        }
        // A `>` means a selected marker cell; skip it and the space after it.
        if i < chars.len() && chars[i] == '>' {
            i += MARKER_W;
        }
        crate::tui::text::display_width(&chars[..i].iter().collect::<String>())
    }

    #[test]
    fn closed_popup_renders_nothing() {
        let p = Palette::default();
        let popup = CompletionPopup::default();
        assert!(render(&popup, 80, 10, 0, &p).lines.is_empty());
    }

    #[test]
    fn lines_are_in_natural_order() {
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        popup.open(vec![cand("alpha", false), cand("beta", false)], 0, 2);
        let v = render(&popup, 40, 10, 0, &p);
        assert_eq!(v.lines.len(), 2);
        // Keep screen order: the first entry on top
        assert!(text_of(&v.lines[0]).contains("alpha"));
        assert!(text_of(&v.lines[1]).contains("beta"));
    }

    #[test]
    fn command_names_start_at_the_anchor_column() {
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        popup.open(
            vec![cmd("/model", "set model"), cmd("/switch", "switch")],
            0,
            6,
        );
        let anchor = 9;
        let v = render(&popup, 80, 10, anchor, &p);
        for l in &v.lines {
            assert_eq!(
                name_col_of(l),
                anchor,
                "命令名必须起于锚点: row={:?}",
                text_of(l)
            );
        }
    }

    #[test]
    fn command_descriptions_start_at_one_common_column() {
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        // 名字长度不同，描述必须被垫到同一列。
        popup.open(
            vec![cmd("/q", "quit"), cmd("/model", "set model")],
            0,
            6,
        );
        let v = render(&popup, 80, 10, 5, &p);
        let col_of = |l: &Line, needle: &str| -> usize {
            let text = text_of(l);
            let byte = text.find(needle).expect("描述必须在行内");
            crate::tui::text::display_width(&text[..byte])
        };
        let a = col_of(&v.lines[0], "quit");
        let b = col_of(&v.lines[1], "set model");
        assert_eq!(a, b, "所有描述首字符必须同列");
    }

    #[test]
    fn plain_rows_start_at_the_anchor_column() {
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        popup.open(vec![cand("src/", true), cand("target/", true)], 0, 5);
        let anchor = 7;
        let v = render(&popup, 80, 10, anchor, &p);
        for l in &v.lines {
            assert_eq!(
                name_col_of(l),
                anchor,
                "路径候选必须起于锚点: {:?}",
                text_of(l)
            );
        }
    }

    #[test]
    fn every_row_carries_the_marker_left_of_the_anchor() {
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        popup.open(vec![cand("alpha", false), cmd("/q", "quit")], 0, 2);
        popup.move_selection(1);
        let anchor = 6;
        let v = render(&popup, 80, 10, anchor, &p);
        // 选中行 `>` 落在 anchor-2
        assert_eq!(text_of(&v.lines[1]).find('>'), Some(anchor - MARKER_W));
        // 未选中行同位置是空格，名字仍在锚点
        assert_eq!(text_of(&v.lines[0]).find('>'), None);
        assert_eq!(name_col_of(&v.lines[0]), anchor);
    }

    #[test]
    fn command_description_is_muted_and_upright() {
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        popup.open(vec![cmd("/model", "set model")], 0, 6);
        let v = render(&popup, 80, 10, 4, &p);
        let desc = v.lines[0]
            .spans
            .iter()
            .find(|s| s.content.contains("set model"))
            .expect("必须有描述 span");
        assert_eq!(desc.style.fg, Some(p.muted), "描述用 muted 灰");
        assert!(
            !desc.style.add_modifier.contains(ratatui::style::Modifier::ITALIC),
            "描述不带斜体"
        );
    }

    #[test]
    fn marker_is_never_accented() {
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        popup.open(vec![cand("alpha", false)], 0, 2);
        popup.move_selection(0);
        let v = render(&popup, 40, 10, 4, &p);
        let marker = v.lines[0]
            .spans
            .iter()
            .find(|s| s.content.contains('>'))
            .expect("选中行必须有 marker span");
        assert_eq!(marker.style.fg, None, "> 不用 accent");
    }

    #[test]
    fn command_name_is_fully_accented_regardless_of_hits() {
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        // 命令名整条都是 accent：打 "/mo" 也是整个 "/model" 上色，
        // accent 在这里的作用是与后面的描述区分，不是标命中。
        let mut c = cmd("/model", "set model");
        c.hits = vec![0, 1, 2];
        popup.open(vec![c], 0, 4);
        let v = render(&popup, 80, 10, 4, &p);
        let name_span = v.lines[0]
            .spans
            .iter()
            .find(|s| s.content.contains("/model"))
            .expect("必须有命令名 span");
        assert_eq!(name_span.content.as_ref(), "/model", "命令名不该被切成多段");
        assert_eq!(name_span.style.fg, Some(p.accent), "命令名整条 accent");
    }

    #[test]
    fn hit_characters_are_accented_and_the_rest_are_not() {
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        // "session/" 的前两个字符命中（打的是 "se"）
        popup.open(vec![path_cand("session/", &[0, 1])], 0, 3);
        let v = render(&popup, 80, 10, 4, &p);
        let span_with = |needle: &str| -> ratatui::style::Style {
            v.lines[0]
                .spans
                .iter()
                .find(|s| s.content.contains(needle))
                .map(|s| s.style)
                .unwrap_or_else(|| panic!("没有含 {needle} 的 span"))
        };
        assert_eq!(span_with("se").fg, Some(p.accent), "命中的 se 用 accent");
        assert_eq!(span_with("ssion").fg, None, "没命中的部分用正常色");
    }

    #[test]
    fn directory_slash_is_never_accented() {
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        // 目录名整个都算命中（打 "session"），但尾部 / 仍不上色
        popup.open(vec![path_cand("session/", &(0..8).collect::<Vec<_>>())], 0, 9);
        let v = render(&popup, 80, 10, 4, &p);
        let slash = v.lines[0]
            .spans
            .iter()
            .find(|s| s.content.contains('/'))
            .expect("目录行必须有 / span");
        assert_eq!(slash.style.fg, None, "目录斜杠永不上色");
    }

    #[test]
    fn anchor_shifts_left_on_overflow() {
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        let long = "y".repeat(50);
        popup.open(vec![cand(&long, false)], 0, 1);
        // 锚点 70 + 内容 > 40 列终端 → 左移，右缘贴内边
        let v = render(&popup, 40, 10, 70, &p);
        assert!(v.anchor_col < 70, "溢出必须左移: anchor={}", v.anchor_col);
    }

    #[test]
    fn very_long_name_does_not_break_alignment() {
        use unicode_width::UnicodeWidthStr;
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        let long = "x".repeat(200);
        popup.open(vec![cand(&long, false)], 0, 1);
        let v = render(&popup, 40, 10, 0, &p);
        let w: usize = v.lines[0].spans.iter().map(|s| s.content.width()).sum();
        assert!(w >= 40, "超长名字不 panic，宽度 {w}");
    }

    #[test]
    fn cjk_names_render_correctly() {
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        popup.open(vec![cand("中文目录/", true)], 0, 1);
        let v = render(&popup, 40, 10, 0, &p);
        assert!(text_of(&v.lines[0]).contains("中文目录/"));
    }

    #[test]
    fn cjk_anchor_uses_display_columns() {
        let p = Palette::default();
        let mut popup = CompletionPopup::default();
        popup.open(vec![cand("中文目录/", true)], 0, 1);
        let anchor = 6;
        let v = render(&popup, 80, 10, anchor, &p);
        assert_eq!(name_col_of(&v.lines[0]), anchor, "CJK 行缩进按显示宽度算");
    }
}
