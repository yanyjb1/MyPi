//! The model name — `[M] <display name>`.

use ratatui::text::Span;

use crate::ui::tui::zone::Ctx;
use crate::ui::tui::zone::main::input::statusline::theme::{StatusTheme, Token};
use crate::ui::tui::zone::main::input::statusline::{Deco, Setup};

#[derive(Debug, Default)]
pub struct Model;

pub(super) fn make(_: &Setup) -> Box<dyn Deco> {
    Box::new(Model::default())
}

impl Deco for Model {
    fn render(&self, t: &StatusTheme, _budget: Option<usize>, ctx: &Ctx) -> Vec<Span<'static>> {
        let Some(name) = ctx.view.model else {
            return Vec::new();
        };
        vec![t.capsule(format!("[M] {name}"), t.get(Token::Accent))]
    }
}
