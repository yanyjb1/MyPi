//! The model name — `[M] <display name>`.

use ratatui::text::Span;

use super::super::theme::{StatusTheme, Token};
use super::super::{Deco, Setup};
use crate::ui::tui::zone::main::input::Ctx;

#[derive(Debug, Default)]
pub struct Model;

pub(super) fn make(_: &Setup) -> Box<dyn Deco> {
    Box::new(Model::default())
}

impl Deco for Model {
    fn render(&self, t: &StatusTheme, _budget: Option<usize>, ctx: &Ctx) -> Vec<Span<'static>> {
        let Some(name) = ctx.data.model.as_deref() else {
            return Vec::new();
        };
        vec![t.capsule(format!("[M] {name}"), t.get(Token::Accent))]
    }
}
