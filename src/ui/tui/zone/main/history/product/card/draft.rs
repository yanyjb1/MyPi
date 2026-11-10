//! final: the draft a subagent leaves behind, prefixing the reused forms.

use super::super::super::row::{Lead, Row};
use super::{
    Section, State, ToolBlock, ToolOutcome, ToolRenderer, arg_str, edit, error_under, file, head,
    output_lines,
};

/// The one-line form; only `submit` has one.
pub(crate) fn inline(args: &str, result: Option<ToolOutcome<'_>>) -> Option<Vec<Row>> {
    if arg_str(args, "type").as_deref() != Some("submit") {
        return None;
    }
    let state = State::of(result);
    let mut out = vec![Row::of(Lead::None, head(state, "Submit"))];
    out.extend(error_under(result));
    Some(out)
}

pub(crate) struct Draft;

impl ToolRenderer for Draft {
    fn label(&self, b: &ToolBlock<'_>) -> Option<String> {
        let op = arg_str(b.args, "type")?;
        let mut chars = op.chars();
        let first = chars.next()?.to_uppercase().to_string();
        Some(format!("Final-{first}{}", chars.as_str()))
    }

    fn call_body(&self, b: &ToolBlock<'_>) -> Vec<Row> {
        match arg_str(b.args, "type").as_deref() {
            Some("write") => file::write_preview(b),
            Some("edit") => edit::Edit.call_body(&ToolBlock { result: None, ..*b }),
            _ => Vec::new(),
        }
    }

    fn result_sections(&self, b: &ToolBlock<'_>, o: ToolOutcome<'_>) -> Vec<Section> {
        let lines = match arg_str(b.args, "type").as_deref() {
            Some("read") => file::read_body(o),
            Some("edit") => return edit::Edit.result_sections(b, o),
            _ => output_lines(o.text),
        };
        vec![Section { label: None, lines }]
    }
}
