//! Input claims and the seats that hold them.

use crate::ui::tui::action::Key;

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

/// Which sub-zone a claim belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seat {
    History,
    Input,
    Reserved,
}
