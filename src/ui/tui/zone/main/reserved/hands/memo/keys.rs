//! The panel's keys: arrows, vim, wasd, Enter, Shift+Enter, Esc.

use crate::ui::tui::action::{Key, KeyCode, Mods};
use crate::ui::tui::zone::Claim;

/// No modifiers held.
const PLAIN: Mods = Mods {
    ctrl: false,
    alt: false,
    shift: false,
};

const fn key(code: KeyCode) -> Claim {
    Claim::Key(Key { code, mods: PLAIN })
}

const fn shifted(code: KeyCode) -> Claim {
    Claim::Key(Key {
        code,
        mods: Mods {
            ctrl: false,
            alt: false,
            shift: true,
        },
    })
}

/// Keys the panel borrows while it is on duty.
pub const PANEL: &[Claim] = &[
    key(KeyCode::Esc),
    key(KeyCode::Up),
    key(KeyCode::Down),
    key(KeyCode::Left),
    key(KeyCode::Right),
    key(KeyCode::Enter),
    shifted(KeyCode::Enter),
    key(KeyCode::Char('k')),
    key(KeyCode::Char('j')),
    key(KeyCode::Char('w')),
    key(KeyCode::Char('s')),
    key(KeyCode::Char('h')),
    key(KeyCode::Char('l')),
    key(KeyCode::Char('a')),
    key(KeyCode::Char('d')),
];
