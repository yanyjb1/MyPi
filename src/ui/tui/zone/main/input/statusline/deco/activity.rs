//! The row's head: `π` idle, spinning while busy.

use ratatui::text::Span;
use serde_json::Value;

use super::super::Deco;
use super::super::theme::{Slot, StatusTheme};
use crate::ui::tui::zone::main::input::Ctx;

/// Animation frames.
const SPINNER: [char; 4] = ['|', '/', '-', '\\'];

#[derive(Debug, Default)]
pub struct Activity {
    frame: usize,
}

pub(super) fn make() -> Box<dyn Deco> {
    Box::new(Activity::default())
}

impl Deco for Activity {
    /// One live increment = one frame. The glyph only shows while the session is busy.
    fn on_event(&mut self, event: &str, _content: &Value, _ctx: &Ctx) {
        if event == crate::ui::tui::zone::name::TICK {
            self.frame = (self.frame + 1) % SPINNER.len();
        }
    }

    fn render(&self, t: &StatusTheme, _budget: Option<usize>, ctx: &Ctx) -> Vec<Span<'static>> {
        if ctx.data.busy {
            let glyph = SPINNER[self.frame].to_string();
            return vec![t.capsule(glyph, t.get(Slot::Accent))];
        }
        let glyph = "π";
        vec![t.capsule(glyph.to_string(), t.get(Slot::Muted))]
    }
}
