//! The `π` mark — idle, or spinning while a turn is in flight.
//!
//! Update source: `ActivityStarted` / `ActivityStopped` from the render loop's
//! busy-edge detection, plus a `Tick` per loop pass. The frames advance only
//! while active; the loop's wake cadence *is* the animation cadence (a silent
//! thinking phase still ticks, because the loop's timeout keeps it alive).

use ratatui::text::Span;

use crate::tui::zone::main::input::statusline::theme::{ColorPolicy, StatusTheme, Token};
use crate::tui::zone::main::input::statusline::{PRIORITY_BASE, Side, StatusComponent, StatusEvent};

/// Animation frames.
const SPINNER: [char; 4] = ['|', '/', '-', '\\'];

#[derive(Debug, Default)]
pub struct Activity {
    active: bool,
    frame: usize,
}

impl StatusComponent for Activity {
    fn id(&self) -> &'static str {
        "activity"
    }

    fn side(&self) -> Side {
        Side::Left
    }

    fn order(&self) -> u8 {
        0
    }

    fn priority(&self) -> u8 {
        PRIORITY_BASE
    }

    fn colors(&self) -> ColorPolicy {
        ColorPolicy::Tokens(&[Token::Muted, Token::Accent, Token::Capsule])
    }

    fn on_event(&mut self, ev: &StatusEvent<'_>) {
        match ev {
            StatusEvent::ActivityStarted => {
                self.active = true;
                self.frame = 0;
            }
            StatusEvent::ActivityStopped => self.active = false,
            StatusEvent::Tick if self.active => {
                self.frame = (self.frame + 1) % SPINNER.len();
            }
            _ => {}
        }
    }

    fn render(&self, t: &StatusTheme, _budget: Option<usize>) -> Vec<Span<'static>> {
        let (glyph, color) = if self.active {
            (SPINNER[self.frame], t.get(Token::Accent))
        } else {
            ('π', t.get(Token::Muted))
        };
        vec![t.capsule(glyph.to_string(), color)]
    }
}
