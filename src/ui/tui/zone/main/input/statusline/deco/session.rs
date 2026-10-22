//! The session name — `< name`, at the right end of the row.

use ratatui::text::Span;

use crate::ui::tui::zone::Ctx;
use crate::ui::tui::zone::main::input::statusline::theme::{StatusTheme, Token};
use crate::ui::tui::zone::main::input::statusline::{Deco, Setup};

#[derive(Debug, Default)]
pub struct Session;

pub(super) fn make(_: &Setup) -> Box<dyn Deco> {
    Box::new(Session::default())
}

impl Deco for Session {
    fn render(&self, t: &StatusTheme, _budget: Option<usize>, ctx: &Ctx) -> Vec<Span<'static>> {
        let Some(name) = ctx.view.name else {
            return Vec::new();
        };
        vec![t.capsule(name.to_string(), t.get(Token::Accent))]
    }
}
