//! The input claims: one concrete key, or a class of input.

use crate::ui::tui::action::{Action, Key};

/// A zone's claim on input: one concrete key, or a class.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Claim {
    Key(Key),
    /// Any printable character.
    AnyChar,
    /// A bracketed paste.
    Paste,
    /// A wheel notch, either direction.
    Wheel,
    /// A wheel notch with shift held.
    WheelShift,
}

impl Claim {
    /// The claim one action falls under.
    pub fn of(action: &Action) -> Claim {
        match action {
            Action::Key(key) => Claim::Key(*key),
            Action::Paste(_) => Claim::Paste,
            Action::ScrollUp { shift: false } | Action::ScrollDown { shift: false } => Claim::Wheel,
            Action::ScrollUp { shift: true } | Action::ScrollDown { shift: true } => {
                Claim::WheelShift
            }
        }
    }
}
