//! The input claims: one concrete key, or a class of input.

use crate::ui::tui::action::{Action, Key, KeyCode};

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

/// Whether one claim covers this action. Classes match their members; this is the only
/// place a claim is interpreted.
pub fn matches(claim: &Claim, a: &Action) -> bool {
    match claim {
        Claim::Key(k) => matches!(a, Action::Key(got) if got == k),
        Claim::AnyChar => matches!(
            a,
            Action::Key(Key {
                code: KeyCode::Char(_),
                ..
            })
        ),
        Claim::Paste => matches!(a, Action::Paste(_)),
        Claim::Wheel => matches!(
            a,
            Action::ScrollUp { shift: false } | Action::ScrollDown { shift: false }
        ),
        Claim::WheelShift => matches!(
            a,
            Action::ScrollUp { shift: true } | Action::ScrollDown { shift: true }
        ),
    }
}
