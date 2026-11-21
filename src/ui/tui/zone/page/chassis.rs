// The page chassis: a titled edge, the caption and the filter box, the body, extra words, the keys.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::ui::tui::text::card::{card_edge, card_edge_labeled, card_row, clip_spans, dashes};
use crate::ui::tui::theme::{Accent, fixed};
use crate::ui::tui::zone::TermSize;

// Rows the frame itself takes: the two edges.
const FRAME: usize = 2;

// Rows the caption takes: its own line, plus a blank either side when it is granted air.
const CAPTION: usize = 1;

// Rows the filter box always takes: the rule over it, its row, the rule under it.
const BOX: usize = 3;

// Rows one grant of air costs: the blank over a block and the blank under it.
const AIR: usize = 2;

// The narrowest body worth keeping; no grant is made that would leave fewer rows than this.
const FLOOR: usize = 8;

// The rule that closes the data and opens the keys.
const RULE: usize = 1;

// The separator between two hints on one row.
const GAP: &str = " · ";

// Columns `GAP` takes.
const GAP_COLS: usize = 3;

// One key hint: the keys to press, and what they do.
pub struct Hint {
    pub keys: &'static str,
    pub what: String,
}

impl Hint {
    pub fn new(keys: &'static str, what: String) -> Hint {
        Hint { keys, what }
    }

    // Columns this hint takes; the separator before it is the row's, not its own.
    fn width(&self) -> usize {
        self.keys.width() + 1 + self.what.width()
    }
}

// The filter box: the reader's own words, or the placeholder while it is empty.
// Its caption sits over the box and says what the filter looks at.
pub struct Filter {
    pub text: String,
    pub placeholder: String,
    pub caption: String,
}

// Everything a full-screen page hands the chassis. Only the content differs between pages.
pub struct Panel {
    pub size: TermSize,
    pub title: String,
    pub filter: Option<Filter>,

    // What the page is for, under the head; absent leaves the head against the data.
    pub note: Option<String>,

    // Lines under the body: the server's opening word, anything the page must say last.
    pub extra: Vec<Line<'static>>,

    // What the page says when the body has nothing; drawn centered.
    pub empty: Option<Line<'static>>,
    pub hints: Vec<Hint>,

    // The live theme color: the title, the filter's prompt and every key are painted with it.
    pub accent: Accent,
}

impl Panel {
    // Blank columns on either side of the content, so the frame is not cramped against it.
    pub fn margin(&self) -> usize {
        margin(self.size)
    }

    // The width a page composes its data rows at: the interior less the two margins.
    pub fn content(&self) -> usize {
        content(self.size)
    }

    // The rows the body may take, everything else deducted.
    pub fn body_rows(&self) -> usize {
        body_rows(
            self.size,
            self.filter.is_some(),
            self.note.is_some(),
            self.extra.len(),
            &self.hints,
        )
    }

    // The whole page, the body drawn where the caller put it.
    pub fn draw(&self, body: Vec<Line<'static>>) -> Vec<Line<'static>> {
        let width = self.size.cols as usize;
        let pad = self.margin();
        let content = self.content();
        let edge = Style::new().fg(fixed::DIM);
        let clear = Style::new();
        let air = air(
            self.size,
            self.filter.is_some(),
            self.note.is_some(),
            self.extra.len(),
            &self.hints,
        );
        let room = air.room();
        let blank = || card_row(Line::default(), edge, clear, width);
        let mut out = Vec::with_capacity(self.size.rows as usize);

        out.push(card_edge_labeled(edge, clear, width, self.head()));

        // The page's own words sit over the data, one blank either side once the air is earned.
        // A note and a caption share that air: a seam between two blocks never takes two blanks.
        let mut owed = false;
        if let Some(note) = &self.note {
            if air.note {
                out.push(blank());
            }
            let line = centered(caption(note), content);
            out.push(card_row(inset(line, pad), edge, clear, width));
            owed = air.note;
        }

        // The page's own word about the filter comes over the box: what it looks at, then the box
        // to type in. The key hints wait at the bottom, under the data.
        if let Some(filter) = &self.filter {
            if owed || air.caption {
                out.push(blank());
            }
            owed = false;
            let caption = centered(caption(&filter.caption), content);
            out.push(card_row(inset(caption, pad), edge, clear, width));
            if air.caption {
                out.push(blank());
            }
            out.push(rule(edge, width));
            out.push(card_row(inset(self.query(filter), pad), edge, clear, width));
            out.push(rule(edge, width));
        }

        // Without a filter box the note is closed by a rule itself, so the head keeps its own band.
        if owed {
            out.push(blank());
            out.push(rule(edge, width));
        }

        let rows = self.body_rows();
        let mut taken = 0;
        if body.is_empty() {
            if let Some(empty) = &self.empty {
                let word = centered(empty.clone(), content);
                out.push(card_row(inset(word, pad), edge, clear, width));
                taken = 1;
            }
        } else {
            for line in body.into_iter().take(rows) {
                let line = fitted(line, content);
                out.push(card_row(inset(line, pad), edge, clear, width));
                taken += 1;
            }
        }

        for _ in taken..rows {
            out.push(blank());
        }

        for line in &self.extra {
            let line = fitted(line.clone(), content);
            out.push(card_row(inset(line, pad), edge, clear, width));
        }

        out.push(rule(edge, width));

        if air.keys {
            out.push(blank());
        }
        for row in pack(&self.hints, content, hint_rows(room)) {
            let hint = centered(hint_line(&row, self.accent), content);
            out.push(card_row(inset(hint, pad), edge, clear, width));
        }
        if air.keys {
            out.push(blank());
        }
        out.push(card_edge(edge, clear, width));

        // A terminal shorter than the chassis itself takes the front of it: nothing may overflow.
        out.truncate(self.size.rows as usize);
        out
    }

    // The head line: the page's own words, in the live color.
    fn head(&self) -> Line<'static> {
        if self.title.is_empty() {
            return Line::default();
        }
        Line::from(Span::styled(
            self.title.clone(),
            self.accent.style().add_modifier(Modifier::BOLD),
        ))
    }

    // The filter row: a prompt, then the reader's words or the placeholder.
    fn query(&self, filter: &Filter) -> Line<'static> {
        let prompt = Span::styled("> ", self.accent.style().add_modifier(Modifier::BOLD));
        if filter.text.is_empty() {
            return Line::from(vec![
                prompt,
                Span::styled(filter.placeholder.clone(), Style::new().fg(fixed::DIM)),
            ]);
        }
        Line::from(vec![
            prompt,
            Span::styled(filter.text.clone(), Style::new().fg(fixed::SYSTEM)),
        ])
    }
}

// on the keys before the caption and the note. The box takes none: it belongs against the data.
struct Air {
    keys: bool,
    caption: bool,
    note: bool,
    room: usize,
}

impl Air {
    fn room(&self) -> usize {
        self.room
    }
}

// Rows the body may take, for a caller that needs the figure before it composes a page.
pub fn body_rows(
    size: TermSize,
    with_filter: bool,
    with_note: bool,
    extra: usize,
    hints: &[Hint],
) -> usize {
    let room = air(size, with_filter, with_note, extra, hints).room();
    // One row is always the body's, however short the terminal is.
    room.saturating_sub(pack(hints, content(size), hint_rows(room)).len())
        .max(1)
        .min(room)
}

fn air(size: TermSize, with_filter: bool, with_note: bool, extra: usize, hints: &[Hint]) -> Air {
    let rows = size.rows as usize;
    let cols = content(size);
    // The caption and the box, without any air between them; a note takes one row wherever.
    let top = if with_filter { CAPTION + BOX } else { 0 } + usize::from(with_note);
    // Without air: the two edges, the rule over the keys, the note and the extra lines.
    let rest = rows.saturating_sub(FRAME + RULE + top + extra);
    // Air is granted from the bottom up: the keys, then the caption, then the note.
    let grants = 1 + usize::from(with_filter) + usize::from(with_note);

    // The roomiest layout that still leaves the body its floor; each step down spends two rows.
    for grant in (0..=grants).rev() {
        let room = rest.saturating_sub(AIR * grant);
        let body = room.saturating_sub(pack(hints, cols, hint_rows(room)).len());
        if body >= FLOOR {
            return Air {
                keys: grant >= 1,
                caption: with_filter && grant >= 2,
                note: with_note && grant >= 2 + usize::from(with_filter),
                room,
            };
        }
    }
    Air {
        keys: false,
        caption: false,
        note: false,
        room: rest,
    }
}

// The frame's interior width.
fn inner(size: TermSize) -> usize {
    (size.cols as usize).saturating_sub(4)
}

// Blank columns on either side of the content: none on a narrow terminal, four at most.
fn margin(size: TermSize) -> usize {
    (inner(size) / 20).min(4)
}

// The width a page composes its rows at.
fn content(size: TermSize) -> usize {
    inner(size).saturating_sub(2 * margin(size))
}

// Rows the hints may take: never more than half the room, so the body always keeps the rest.
fn hint_rows(room: usize) -> usize {
    (room / 2).max(1)
}

// The caption over the filter box: what the filter looks at, in the frame's dim ink.
fn caption(text: &str) -> Line<'static> {
    if text.is_empty() {
        return Line::default();
    }
    Line::from(Span::styled(text.to_string(), Style::new().fg(fixed::DIM)))
}

// A rule between two content blocks: the frame's sides, dashed.
fn rule(edge: Style, width: usize) -> Line<'static> {
    if width < 2 {
        return card_edge(edge, Style::new(), width);
    }
    Line::from(vec![
        Span::styled("|", edge),
        Span::styled(dashes(width - 2), edge),
        Span::styled("|", edge),
    ])
}

// A rule between two blocks of one page, for a renderer that cuts its own body in two.
pub fn divider(width: usize) -> Line<'static> {
    rule(Style::new().fg(fixed::DIM), width)
}

// The page's own words sit in the middle of the row; data rows keep their left edge.
fn centered(line: Line<'static>, inner: usize) -> Line<'static> {
    // A word too long for the row is cut here: `card_row` would otherwise leave it unclipped.
    if line.width() > inner {
        return Line::from(clip_spans(line.spans, inner));
    }
    let used = line.width();
    let lead = inner.saturating_sub(used) / 2;
    if lead == 0 {
        return line;
    }
    let mut spans = Vec::with_capacity(line.spans.len() + 1);
    spans.push(Span::raw(" ".repeat(lead)));
    spans.extend(line.spans);
    Line::from(spans)
}

// A data row cut at the content width, so it never runs over the right margin.
fn fitted(line: Line<'static>, inner: usize) -> Line<'static> {
    if line.width() <= inner {
        return line;
    }
    Line::from(clip_spans(line.spans, inner))
}

// The frame's inner margin, put in front of one row.
fn inset(line: Line<'static>, pad: usize) -> Line<'static> {
    if pad == 0 {
        return line;
    }
    let mut spans = Vec::with_capacity(line.spans.len() + 1);
    spans.push(Span::raw(" ".repeat(pad)));
    spans.extend(line.spans);
    Line::from(spans).style(line.style)
}

// Whole hints packed into rows that fit: a hint never splits across two rows.
// `room` bounds the rows all hints may take; a terminal too short for all of them keeps the front.
fn pack(hints: &[Hint], inner: usize, room: usize) -> Vec<Vec<&Hint>> {
    let budget = room.max(1);
    let mut out: Vec<Vec<&Hint>> = Vec::new();
    let mut row: Vec<&Hint> = Vec::new();
    let mut used = 0usize;

    for hint in hints {
        if out.len() >= budget {
            return out;
        }
        let width = hint.width();
        if !row.is_empty() && used + GAP_COLS + width > inner {
            out.push(std::mem::take(&mut row));
            used = 0;
        }
        used += if row.is_empty() {
            width
        } else {
            GAP_COLS + width
        };
        row.push(hint);
    }
    if !row.is_empty() && out.len() < budget {
        out.push(row);
    }
    out
}

// One row of hints: the keys in the live color, what they do dim.
fn hint_line(row: &[&Hint], accent: Accent) -> Line<'static> {
    let mut spans = Vec::new();

    for (at, hint) in row.iter().enumerate() {
        if at > 0 {
            spans.push(Span::styled(GAP, Style::new().fg(fixed::DIM)));
        }
        spans.push(Span::styled(
            hint.keys,
            accent.style().add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            hint.what.clone(),
            Style::new().fg(fixed::MUTED),
        ));
    }

    Line::from(spans)
}
