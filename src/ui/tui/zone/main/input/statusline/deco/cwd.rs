//! The working directory — `[D] <path>`, `[T] <path>` when it moved for one call.

use ratatui::text::Span;

use super::super::Deco;
use super::super::theme::{Slot, StatusTheme};
use crate::ui::tui::text::short;
use crate::ui::tui::zone::main::input::Ctx;

#[derive(Debug)]
pub struct Cwd;

pub(super) fn make() -> Box<dyn Deco> {
    Box::new(Cwd)
}

impl Deco for Cwd {
    fn render(&self, t: &StatusTheme, _budget: Option<usize>, ctx: &Ctx) -> Vec<Span<'static>> {
        if ctx.data.cwd.is_empty() {
            return Vec::new();
        }
        let marker = if ctx.data.temporary { "[T]" } else { "[D]" };
        vec![t.capsule(
            format!("{} {}", marker, short(&ctx.data.cwd)),
            t.get(Slot::Text),
        )]
    }
}
