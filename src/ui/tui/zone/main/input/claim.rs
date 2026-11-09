//! The keys the editor acts on, named one by one.

use crate::ui::tui::action::{Key, KeyCode, Mods};
use crate::ui::tui::zone::Claim;

/// No modifiers held.
const PLAIN: Mods = Mods {
    ctrl: false,
    alt: false,
    shift: false,
};

/// Everything the editor acts on. `AnyChar` covers the plain typing keys; the rest are
/// named so no key is taken by accident.
pub const CLAIMS: &[Claim] = &[
    Claim::AnyChar,
    Claim::Paste,
    Claim::Key(Key {
        code: KeyCode::Enter,
        mods: PLAIN,
    }),
    Claim::Key(Key {
        code: KeyCode::Enter,
        mods: Mods {
            ctrl: false,
            alt: true,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::Enter,
        mods: Mods {
            ctrl: false,
            alt: false,
            shift: true,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::Esc,
        mods: PLAIN,
    }),
    Claim::Key(Key {
        code: KeyCode::Up,
        mods: Mods {
            ctrl: false,
            alt: true,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::Tab,
        mods: PLAIN,
    }),
    Claim::Key(Key {
        code: KeyCode::Backspace,
        mods: PLAIN,
    }),
    Claim::Key(Key {
        code: KeyCode::Delete,
        mods: PLAIN,
    }),
    Claim::Key(Key {
        code: KeyCode::Left,
        mods: PLAIN,
    }),
    Claim::Key(Key {
        code: KeyCode::Right,
        mods: PLAIN,
    }),
    Claim::Key(Key {
        code: KeyCode::Up,
        mods: PLAIN,
    }),
    Claim::Key(Key {
        code: KeyCode::Down,
        mods: PLAIN,
    }),
    Claim::Key(Key {
        code: KeyCode::Home,
        mods: PLAIN,
    }),
    Claim::Key(Key {
        code: KeyCode::End,
        mods: PLAIN,
    }),
    Claim::Key(Key {
        code: KeyCode::Left,
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::Right,
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::Home,
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::End,
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: false,
        },
    }),
];
