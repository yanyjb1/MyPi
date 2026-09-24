//! Zone abstraction — the input-routing contract for the three screen areas
//! (history, input, reserved) plus full-screen modals.
//!
//! A zone declares what events it accepts (`wants`) and handles them,
//! emitting `Cascade`s (cross-zone side effects) instead of touching other
//! zones directly. The app loop stays a thin router: translate → route →
//! apply cascades → redraw.

use crate::tui::keys::Action;

/// Where an event lands. Modals sit above the three base zones and take
/// over the whole screen while active.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneId {
    /// Chat history (scrollback). Mouse: yes. Cursor keys: no.
    History,
    /// User input (editor + completion pipeline). Cursor keys: yes. Mouse: no.
    Input,
    /// Reserved strip under the input (popup list, pickers).
    Reserved,
    /// A modal overlay (resume picker, tree picker...). Highest routing priority.
    Modal(ModalKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModalKind {
    /// /resume session picker.
    Resume,
    /// Conversation tree navigator (double-Esc).
    Tree,
}

/// Cross-zone side effects a zone may request after handling an event.
/// The app applies these in order; zones never mutate each other.
#[derive(Debug, Clone, PartialEq)]
pub enum Cascade {
    /// Layout must be recomputed next frame (popup opened/closed, modal toggled).
    LayoutDirty,
    /// Close the input zone's completion popup (e.g. because a modal opened).
    DismissCompletion,
    /// Editor content changed → re-run the completion pipeline.
    RefreshCompletions,
    /// The user pressed submit in the input zone.
    Submit,
    /// Quit the app.
    Quit,
    /// Interrupt the streaming turn.
    Interrupt,
    /// Open a modal.
    OpenModal(ModalKind),
    /// Close the active modal.
    CloseModal,
}

/// The zone trait. Implementations own their own state; `handle` returns the
/// cascades the app must apply. Keep `wants` a pure predicate — it runs for
/// every event and must not mutate.
pub trait Zone {
    /// Does this zone accept the action given the current global context?
    /// The action-to-zone routing table lives here, per zone, in code —
    /// the readable source of "who takes what".
    fn wants(&self, action: &Action, modal_active: bool) -> bool;

    /// Handle the action and report side effects.
    fn handle(&mut self, action: Action) -> Vec<Cascade>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modal_ids_are_distinct() {
        assert_ne!(
            ZoneId::Modal(ModalKind::Resume),
            ZoneId::Modal(ModalKind::Tree)
        );
        assert_ne!(ZoneId::History, ZoneId::Input);
    }
}
