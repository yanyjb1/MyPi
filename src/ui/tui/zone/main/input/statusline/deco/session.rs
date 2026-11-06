//! The session name — `< name`, at the right end of the row.

use ratatui::text::Span;

use super::super::Deco;
use super::super::theme::{Slot, StatusTheme};
use crate::ui::tui::zone::main::input::Ctx;

/// What the pill says before a session has a name.
const NAMELESS: &str = "MyPi";

#[derive(Debug, Default)]
pub struct Session;

pub(super) fn make() -> Box<dyn Deco> {
    Box::new(Session::default())
}

impl Deco for Session {
    fn render(&self, t: &StatusTheme, _budget: Option<usize>, ctx: &Ctx) -> Vec<Span<'static>> {
        let name = ctx.data.name.as_deref().unwrap_or(NAMELESS);
        vec![t.capsule(name.to_string(), t.get(Slot::Accent))]
    }
}
