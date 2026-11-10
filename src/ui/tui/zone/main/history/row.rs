//! One finished row: text runs, each with its color slot.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::ui::tui::text::display_width;
use crate::ui::tui::theme::{Accent, fixed};

/// Marks a natural line's first row.
pub const MARK: &str = "▮ ";
/// Marks a row that only exists because the terminal is too narrow.
pub const MORE: &str = "↳ ";

/// Where a row sits in its natural line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Lead {
    /// Card rows: the frame supplies its own left edge.
    #[default]
    None,
    First,
    Wrapped,
}

impl Lead {
    /// Columns the lead marker takes.
    pub fn width(self) -> usize {
        match self {
            Lead::None => 0,
            Lead::First | Lead::Wrapped => 2,
        }
    }

    pub fn text(self) -> Option<&'static str> {
        match self {
            Lead::None => None,
            Lead::First => Some(MARK),
            Lead::Wrapped => Some(MORE),
        }
    }
}

/// Where a run's color comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ink {
    Fixed(Color),
    Accent,
    /// The accent mixed toward the card fill; `0` is no mix.
    AccentShade(u8),
}

/// One run of text and how it is colored.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub text: String,
    pub ink: Option<Ink>,
    pub bg: Option<Ink>,
    pub attrs: Modifier,
}

impl Run {
    pub fn fixed(text: impl Into<String>, color: Color) -> Run {
        Run {
            text: text.into(),
            ink: Some(Ink::Fixed(color)),
            bg: None,
            attrs: Modifier::empty(),
        }
    }

    pub fn accent(text: impl Into<String>) -> Run {
        Run {
            text: text.into(),
            ink: Some(Ink::Accent),
            bg: None,
            attrs: Modifier::empty(),
        }
    }

    /// The terminal's own foreground.
    pub fn plain(text: impl Into<String>) -> Run {
        Run {
            text: text.into(),
            ink: None,
            bg: None,
            attrs: Modifier::empty(),
        }
    }

    pub fn with(text: impl Into<String>, ink: Option<Ink>, attrs: Modifier) -> Run {
        Run {
            text: text.into(),
            ink,
            bg: None,
            attrs,
        }
    }

    /// The fill behind this run.
    pub fn fill(mut self, bg: Ink) -> Run {
        self.bg = Some(bg);
        self
    }

    pub fn bold(self) -> Run {
        Run {
            attrs: self.attrs | Modifier::BOLD,
            ..self
        }
    }

    pub fn width(&self) -> usize {
        display_width(&self.text)
    }
}

/// One finished row.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Row {
    pub lead: Lead,
    pub runs: Vec<Run>,
}

impl Row {
    pub fn new(lead: Lead) -> Row {
        Row {
            lead,
            runs: Vec::new(),
        }
    }

    pub fn of(lead: Lead, runs: Vec<Run>) -> Row {
        Row { lead, runs }
    }

    pub fn text(lead: Lead, text: impl Into<String>, ink: Option<Ink>) -> Row {
        Row::of(
            lead,
            vec![Run {
                text: text.into(),
                ink,
                bg: None,
                attrs: Modifier::empty(),
            }],
        )
    }

    pub fn blank() -> Row {
        Row::new(Lead::None)
    }

    pub fn push(&mut self, run: Run) {
        self.runs.push(run);
    }

    pub fn is_blank(&self) -> bool {
        self.runs.iter().all(|run| run.text.is_empty())
    }

    /// Cells this row takes, lead included.
    pub fn width(&self) -> usize {
        self.lead.width() + self.runs.iter().map(Run::width).sum::<usize>()
    }

    /// This row in the accent's colors.
    pub fn paint(&self, accent: Accent) -> Line<'static> {
        let mut spans: Vec<Span<'static>> = Vec::new();
        if let Some(mark) = self.lead.text() {
            spans.push(Span::styled(mark, Style::new().fg(fixed::DIM)));
        }
        for run in &self.runs {
            let mut style = Style::new().add_modifier(run.attrs);
            style = match run.ink {
                Some(Ink::Fixed(color)) => style.fg(color),
                Some(Ink::Accent) => style.fg(accent.color()),
                Some(Ink::AccentShade(level)) => style.fg(accent.shade(level)),
                None => style,
            };
            style = match run.bg {
                Some(Ink::Fixed(color)) => style.bg(color),
                Some(Ink::Accent) => style.bg(accent.color()),
                Some(Ink::AccentShade(_)) => style,
                None => style,
            };
            spans.push(Span::styled(run.text.clone(), style));
        }
        Line::from(spans)
    }
}

/// Continue a lead on the next row of the same natural line.
fn cont(lead: Lead) -> Lead {
    match lead {
        Lead::None => Lead::None,
        Lead::First | Lead::Wrapped => Lead::Wrapped,
    }
}

/// Split one natural line's runs at the given char indices, one row each.
pub fn split(runs: &[Run], breaks: &[usize], lead: Lead) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::new();
    let mut cur = Row::new(lead);
    let mut at = 0usize;
    let mut cut = 0usize;
    let mut next = breaks.first().copied().unwrap_or(usize::MAX);
    for run in runs {
        let mut buf = String::new();
        for ch in run.text.chars() {
            if at == next && (!buf.is_empty() || !cur.runs.is_empty()) {
                if !buf.is_empty() {
                    cur.push(Run {
                        text: std::mem::take(&mut buf),
                        ..run.clone()
                    });
                }
                out.push(std::mem::replace(&mut cur, Row::new(cont(lead))));
                cut += 1;
                next = breaks.get(cut).copied().unwrap_or(usize::MAX);
            }
            buf.push(ch);
            at += 1;
        }
        if !buf.is_empty() {
            cur.push(Run {
                text: buf,
                ..run.clone()
            });
        }
    }
    if !cur.runs.is_empty() {
        out.push(cur);
    }
    if out.is_empty() {
        out.push(Row {
            lead,
            runs: runs.to_vec(),
        });
    }
    out
}
