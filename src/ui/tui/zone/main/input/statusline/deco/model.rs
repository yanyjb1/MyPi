//! The model name — `[M] <display name>`.

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
        let text = match ctx.data.provider.as_deref() {
            Some(provider) => format!("[M] {name} ({provider})"),
            None => format!("[M] {name}"),
        };
        vec![t.capsule(text, t.get(Slot::Accent))]
    }
}
