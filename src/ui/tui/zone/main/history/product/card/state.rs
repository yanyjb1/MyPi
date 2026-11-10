//! How a card is colored, by the state of the exchange.

use super::super::super::row::Ink;
use super::ToolOutcome;
use crate::ui::tui::text::glyphs as g;
use crate::ui::tui::theme::fixed;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum State {
    /// No settled result yet: the call is queued, its arguments still arrive, or its output streams.
    Pending,
    Ok,
    Err,
}

impl State {
    pub(crate) fn of(result: Option<ToolOutcome<'_>>) -> State {
        match result.and_then(|o| o.ok) {
            Some(true) => State::Ok,
            Some(false) => State::Err,
            None => State::Pending,
        }
    }

    pub(crate) fn glyph(self) -> &'static str {
        match self {
            State::Pending => g::PENDING,
            State::Ok => g::OK,
            State::Err => g::ERR,
        }
    }

    pub(crate) fn glyph_ink(self) -> Ink {
        match self {
            State::Pending => Ink::Accent,
            State::Ok => Ink::Fixed(fixed::OK),
            State::Err => Ink::Fixed(fixed::ERR),
        }
    }

    pub(super) fn edge(self) -> Ink {
        match self {
            State::Pending => Ink::Accent,
            State::Ok => Ink::Fixed(fixed::TOOL_EDGE_OK),
            State::Err => Ink::Fixed(fixed::TOOL_EDGE_ERR),
        }
    }

    pub(super) fn bg(self) -> Ink {
        Ink::Fixed(match self {
            State::Pending => fixed::TOOL_BG_PENDING,
            State::Ok => fixed::TOOL_BG_OK,
            State::Err => fixed::TOOL_BG_ERR,
        })
    }
}
