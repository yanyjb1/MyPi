//! The working directory — `[D] <path>`, `[T] <path>` when it moved for one call.

use std::path::Path;

use ratatui::text::Span;

use super::super::theme::{StatusTheme, Token};
use super::super::{Deco, Setup};
use crate::ui::tui::zone::main::input::Ctx;

#[derive(Debug, Default)]
pub struct Cwd;

pub(super) fn make(_: &Setup) -> Box<dyn Deco> {
    Box::new(Cwd::default())
}

impl Deco for Cwd {
    fn render(&self, t: &StatusTheme, _budget: Option<usize>, ctx: &Ctx) -> Vec<Span<'static>> {
        if ctx.data.cwd.is_empty() {
            return Vec::new();
        }
        let marker = if ctx.data.temporary { "[T]" } else { "[D]" };
        vec![t.capsule(
            format!("{} {}", marker, short_path(Path::new(&ctx.data.cwd))),
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
