//! The session name — `< name`, at the right end of the row.

use ratatui::text::Span;

use super::super::theme::{Slot, StatusTheme};
use super::super::{Deco, Setup};
use crate::ui::tui::zone::main::input::Ctx;

#[derive(Debug, Default)]
pub struct Session;

pub(super) fn make(_: &Setup) -> Box<dyn Deco> {
    Box::new(Session::default())
}

impl Deco for Session {
    fn render(&self, t: &StatusTheme, _budget: Option<usize>, ctx: &Ctx) -> Vec<Span<'static>> {
        let Some(name) = ctx.data.name.as_deref() else {
            return Vec::new();
        };
        vec![t.capsule(name.to_string(), t.get(Slot::Accent))]
    }
}
