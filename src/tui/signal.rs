//! Signal protocol — everything that can dirty a region, on one channel.
//!
//! The main loop is **signal-driven, not polled**: an input thread
//! forwards crossterm events, the server facade forwards `SessionEvent`s,
//! and the loop blocks on `recv()` until something arrives. Zero CPU
//! while idle. A dirty region is marked idempotently — a burst of
//! deltas coalesces into one repaint — and ratatui's internal double
//! buffer diff means only changed cells reach the terminal.

use crate::server::events::SessionEvent;

/// One unit of "something happened". Regions decide for themselves
/// whether a signal dirties them; the loop never inspects the payload.
#[derive(Debug, Clone)]
pub enum Signal {
    /// A semantic action translated from keyboard/paste input (input
    /// thread → main loop; translation needs the current KeyContext,
    /// which only the main thread can build).
    ///
    /// Why translate in the main thread: `translate_with` must see
    /// `KeyContext` (popup open? streaming?) — stale context in the
    /// sender would misroute keys. The thread forwards raw events; the
    /// loop translates at the moment of consumption.
    Key(ratatui::crossterm::event::KeyEvent),
    /// Paste arrives as its own event (bracketed paste).
    Paste(String),
    /// Mouse event; routed by hit-testing the last frame's layout.
    Mouse(ratatui::crossterm::event::MouseEvent),
    /// Terminal resized: all width-dependent caches are invalid.
    Resized,
    /// The turn runner produced an event (delta / tool / commit / done).
    Session(SessionEvent),
}

impl Signal {
    /// True when the signal is a keyboard or paste action — the loop
    /// routes these through `App::apply`, everything else is advisory
    /// (mark dirty + collect cascades).
    pub fn is_action(&self) -> bool {
        matches!(self, Signal::Key(_) | Signal::Paste(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_are_keys_and_pastes() {
        use ratatui::crossterm::event::{KeyCode, KeyEvent as K, KeyModifiers};
        let k = K::new(KeyCode::Char('a'), KeyModifiers::NONE);
        assert!(Signal::Key(k).is_action());
        assert!(Signal::Paste("x".into()).is_action());
        assert!(!Signal::Resized.is_action());
        assert!(!Signal::Session(SessionEvent::Done).is_action());
    }
}
