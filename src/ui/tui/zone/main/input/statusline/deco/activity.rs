//! The row's head: `π` idle, `Tel` with a bridge, spinning while busy.

use ratatui::text::Span;

use crate::ui::tui::zone::main::input::statusline::theme::{StatusTheme, Token};
use crate::ui::tui::zone::main::input::statusline::{Deco, Setup};
use crate::ui::tui::zone::{Ctx, Event};

/// Animation frames.
const SPINNER: [char; 4] = ['|', '/', '-', '\\'];

#[derive(Debug, Default)]
pub struct Activity {
    frame: usize,
}

pub(super) fn make(_: &Setup) -> Box<dyn Deco> {
    Box::new(Activity::default())
}

impl Deco for Activity {
    fn on_event(&mut self, ev: Event, _ctx: &Ctx) {
        if matches!(ev, Event::Tick) {
            self.frame = (self.frame + 1) % SPINNER.len();
        }
    }

    fn render(&self, t: &StatusTheme, _budget: Option<usize>, ctx: &Ctx) -> Vec<Span<'static>> {
        if ctx.view.busy {
            let glyph = SPINNER[self.frame].to_string();
            return vec![t.capsule(glyph, t.get(Token::Accent))];
        }
        let glyph = if ctx.view.bridge.is_some() {
            "Tel"
        } else {
            "π"
        };
        vec![t.capsule(glyph.to_string(), t.get(Token::Muted))]
    }
}
