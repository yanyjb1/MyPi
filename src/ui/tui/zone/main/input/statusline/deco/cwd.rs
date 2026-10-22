//! The working directory — `[D] <path>`, `[T] <path>` when migrated.

use std::path::Path;

use ratatui::text::Span;

use crate::ui::tui::zone::Ctx;
use crate::ui::tui::zone::main::input::statusline::theme::{StatusTheme, Token};
use crate::ui::tui::zone::main::input::statusline::{Deco, Setup};

#[derive(Debug, Default)]
pub struct Cwd;

pub(super) fn make(_: &Setup) -> Box<dyn Deco> {
    Box::new(Cwd::default())
}

impl Deco for Cwd {
    fn render(&self, t: &StatusTheme, _budget: Option<usize>, ctx: &Ctx) -> Vec<Span<'static>> {
        if ctx.view.cwd.is_empty() {
            return Vec::new();
        }
        let marker = if ctx.view.cwd_temporary { "[T]" } else { "[D]" };
        vec![t.capsule(
            format!("{} {}", marker, short_path(Path::new(ctx.view.cwd))),
            t.get(Token::Text),
        )]
    }
}

/// The last two path components: `/home/Arisha/Utility/MyPi` -> `Utility/MyPi`.
fn short_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    let parts: Vec<&str> = s.split('/').filter(|s| !s.is_empty()).collect();
    match parts.len() {
        0 => "/".into(),
        1 => format!("/{}", parts[0]),
        _ => format!("{}/{}", parts[parts.len() - 2], parts[parts.len() - 1]),
    }
}
