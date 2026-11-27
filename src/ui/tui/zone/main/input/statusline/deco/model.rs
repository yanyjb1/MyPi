// The model pill — `[<effort>] <display name> (<provider>)`, `[M]` when the wire names none.

use ratatui::text::Span;

use super::super::Deco;
use super::super::theme::{Slot, StatusTheme};
use crate::ui::tui::zone::main::input::Ctx;

#[derive(Debug, Default)]
pub struct Model;

pub(super) fn make() -> Box<dyn Deco> {
    Box::new(Model)
}

impl Deco for Model {
    fn render(&self, t: &StatusTheme, _budget: Option<usize>, ctx: &Ctx) -> Vec<Span<'static>> {
        let Some(name) = ctx.data.model.as_deref() else {
            return Vec::new();
        };
        // The brackets always stand; inside is the rung, `auto` while it is unset, or `M` for a wire that names none.
        let tag = match ctx.data.takes_effort() {
            false => "M",
            true if ctx.data.effort.is_empty() => "auto",
            true => ctx.data.effort.as_str(),
        };
        let text = match ctx.data.provider.as_deref() {
            Some(provider) => format!("[{tag}] {name} ({provider})"),
            None => format!("[{tag}] {name}"),
        };
        vec![t.capsule(text, t.get(Slot::Accent))]
    }
}
