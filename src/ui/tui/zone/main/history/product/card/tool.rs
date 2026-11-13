//! The renderer contract: one exchange as the renderers see it, and the family table.

use ratatui::style::Modifier;
use serde_json::Value;

use super::super::super::row::{Ink, Lead, Row, Run};
use super::frame::{default_call, error_body, non_empty_lines, one_line, output_lines};
use super::state::State;
use super::{bash, context, draft, edit, fallback, file, kairos, mcp, memo, web};
use crate::ui::tui::text::duration;
use crate::ui::tui::text::glyphs as g;
use crate::ui::tui::theme::fixed;

/// What a paired result says about the call.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ToolOutcome<'a> {
    pub text: &'a str,
    /// `None` while the call is still running.
    pub ok: Option<bool>,
    pub details: Option<&'a Value>,
    pub duration_ms: u64,
}

impl ToolOutcome<'_> {
    fn duration_label(&self) -> Option<String> {
        (self.duration_ms >= 1_000).then(|| duration(self.duration_ms))
    }
}

/// One tool exchange as the renderers see it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ToolBlock<'a> {
    pub name: &'a str,
    pub args: &'a str,
    pub intent: Option<&'a str>,
    pub result: Option<ToolOutcome<'a>>,
    pub expanded: bool,
    /// Rows a head or a tail may take before it folds.
    pub budget: usize,
    pub width: usize,
}

impl ToolBlock<'_> {
    /// The columns a row may take inside the frame.
    pub(crate) fn inner_width(&self) -> usize {
        self.width.saturating_sub(4)
    }

    fn renderer(&self) -> &'static dyn ToolRenderer {
        renderer_for(self.name)
    }

    fn state(&self) -> State {
        State::of(self.result)
    }

    pub(super) fn edge(&self) -> Ink {
        self.state().edge()
    }

    pub(super) fn bg(&self) -> Ink {
        self.state().bg()
    }

    pub(super) fn header(&self) -> Row {
        let state = self.state();
        let shown = self
            .renderer()
            .label(self)
            .unwrap_or_else(|| self.name.to_string());
        let mut runs = vec![
            Run::with(state.glyph(), Some(state.glyph_ink()), Modifier::empty()),
            Run::fixed(format!(" {shown}"), fixed::TOOL_TITLE).bold(),
        ];
        if let Some(title) = self.renderer().title(self) {
            runs.push(Run::fixed(format!(": {title}"), fixed::MUTED));
            return Row::of(Lead::None, runs);
        }
        if let Some(intent) = self.intent.filter(|intent| !intent.trim().is_empty()) {
            runs.push(Run::fixed(format!(": {}", one_line(intent)), fixed::MUTED));
        }
        if let Some(path) = self.renderer().path(self) {
            runs.push(Run::fixed(format!(" {} ", g::SEP), fixed::DIM));
            runs.push(Run::fixed(path, fixed::TOOL_PATH));
        }
        for m in self.meta() {
            runs.push(Run::fixed(format!(" {} {m}", g::SEP), fixed::DIM));
        }
        Row::of(Lead::None, runs)
    }

    fn meta(&self) -> Vec<String> {
        let r = self.renderer();
        let mut out = r.meta(self);
        if let Some(o) = self.result {
            if let Some(m) = r.result_measure(o) {
                out.push(m);
            }
            if let Some(d) = o.duration_label() {
                out.push(d);
            }
        }
        out
    }

    fn call_body(&self) -> Vec<Row> {
        self.renderer().call_body(self)
    }

    fn result_sections(&self, o: ToolOutcome<'_>) -> Vec<Section> {
        self.renderer().result_sections(self, o)
    }

    pub(super) fn sections(&self) -> Vec<Section> {
        let mut out = Vec::new();
        let call = self.call_body();
        if !call.is_empty() {
            out.push(Section {
                label: None,
                lines: call,
            });
        }
        if let Some(o) = self.result {
            out.extend(self.result_sections(o));
        }
        out.retain(|s| !s.lines.is_empty());
        out
    }
}

/// One run of what a card shows, under a label it may carry.
pub(crate) struct Section {
    pub label: Option<&'static str>,
    pub lines: Vec<Row>,
}

/// What one tool family draws for the call and for the result.
pub(crate) trait ToolRenderer {
    /// The name shown in the header, when it is not the tool's own.
    fn label(&self, _b: &ToolBlock<'_>) -> Option<String> {
        None
    }

    /// The text after the header's colon, when the card names it itself.
    fn title(&self, _b: &ToolBlock<'_>) -> Option<String> {
        None
    }

    fn meta(&self, _b: &ToolBlock<'_>) -> Vec<String> {
        Vec::new()
    }

    /// The file this call names, shown in the path color.
    fn path(&self, _b: &ToolBlock<'_>) -> Option<String> {
        None
    }

    fn result_measure(&self, o: ToolOutcome<'_>) -> Option<String> {
        let lines = non_empty_lines(o.text);
        (lines > 1).then(|| format!("{lines} lines"))
    }

    fn call_body(&self, b: &ToolBlock<'_>) -> Vec<Row> {
        default_call(b.name, b.args)
    }

    fn result_sections(&self, b: &ToolBlock<'_>, o: ToolOutcome<'_>) -> Vec<Section> {
        let lines = match o.ok {
            Some(false) => error_body(o.text),
            // A running result shows its tail: build only what the fold keeps.
            None if !b.expanded => super::frame::output_tail(o.text, b.budget),
            _ => output_lines(o.text),
        };
        vec![Section { label: None, lines }]
    }
}

fn renderer_for(name: &str) -> &'static dyn ToolRenderer {
    match name {
        "bash" => &bash::Bash,
        "edit" => &edit::Edit,
        "read" | "write" | "cd" => &file::FileTools,
        "web" => &web::Web,
        "context" => &context::Context,
        "memo" => &memo::Memo,
        "mcp" => &mcp::Call,
        "kairos" => &kairos::Payload,
        "final" => &draft::Draft,
        _ => &fallback::Fallback,
    }
}

/// The status glyph and a bold title, for the one-line forms.
pub(crate) fn head(state: State, label: &str) -> Vec<Run> {
    vec![
        Run::with(state.glyph(), Some(state.glyph_ink()), Modifier::empty()),
        Run::fixed(format!(" {label}"), fixed::TOOL_TITLE).bold(),
    ]
}

/// The head with the first line after it; every later line on a row of its own.
pub(crate) fn head_lines(state: State, label: &str, lines: &[String]) -> Vec<Row> {
    let mut first = head(state, label);
    let mut rest = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if index == 0 {
            first.push(Run::fixed(format!(": {line}"), fixed::MUTED));
        } else {
            rest.push(Row::of(
                Lead::None,
                vec![Run::fixed(format!(": {line}"), fixed::MUTED)],
            ));
        }
    }
    let mut out = vec![Row::of(Lead::None, first)];
    out.extend(rest);
    out
}

/// The failure text under a one-line form: the card body, shifted under the title.
pub(crate) fn error_under(result: Option<ToolOutcome<'_>>) -> Vec<Row> {
    let Some(o) = result.filter(|o| o.ok == Some(false)) else {
        return Vec::new();
    };
    error_body(o.text)
        .into_iter()
        .map(|mut row| {
            if let Some(first) = row.runs.first_mut() {
                first.text.insert_str(0, "  ");
            }
            row
        })
        .collect()
}

/// One diff line: the line it sat on, and its text.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Numbered {
    pub at: Option<usize>,
    pub text: String,
}

/// What an `edit` result leaves for the card to line up: the start line when the request cannot
/// name it, and the replaced text when the request does not carry it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ToolView {
    Plain {
        text: String,
    },
    Diff {
        at: Option<usize>,
        old: Option<String>,
    },
}

impl ToolView {
    pub(crate) fn from_details(details: Option<&Value>, text: &str) -> ToolView {
        let diff = details.filter(|d| d.get("kind").and_then(|k| k.as_str()) == Some("diff"));
        if let Some(d) = diff {
            let at = d.get("atLine").and_then(Value::as_u64).map(|n| n as usize);
            let old = d.get("old").and_then(Value::as_str).map(str::to_string);
            if at.is_some() || old.is_some() {
                return ToolView::Diff { at, old };
            }
        }
        ToolView::Plain {
            text: text.to_string(),
        }
    }
}
