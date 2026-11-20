//! The keys the transcript consumes, and what each one does to the layout.

use super::zone::HistoryZone;
use crate::ui::tui::action::{Key, KeyCode, Mods};
use crate::ui::tui::zone::Claim;

/// The keys history consumes. The input zone yields Ctrl+arrows to it.
pub const CLAIMS: &[Claim] = &[
    Claim::Key(Key {
        code: KeyCode::Char('t'),
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::Char('o'),
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::Up,
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::Down,
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::PageUp,
        mods: Mods {
            ctrl: false,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::PageDown,
        mods: Mods {
            ctrl: false,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::Home,
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: true,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::End,
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: true,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::PageUp,
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: false,
        },
    }),
    Claim::Key(Key {
        code: KeyCode::PageDown,
        mods: Mods {
            ctrl: true,
            alt: false,
            shift: false,
        },
    }),
    Claim::Wheel,
    Claim::WheelShift,
];

/// One claimed key: `true` when the transcript used it.
pub(super) fn press(zone: &mut HistoryZone, key: Key) -> bool {
    match key.code {
        KeyCode::Char('t') if key.mods.ctrl => {
            zone.thinking = !zone.thinking;
            zone.local.drop_products();
            zone.run.clear();
            true
        }
        KeyCode::Char('o') if key.mods.ctrl => {
            zone.tool = !zone.tool;
            zone.local.drop_products();
            zone.run.clear();
            true
        }
        KeyCode::Up if key.mods.ctrl => {
            zone.step_slot(true);
            true
        }
        KeyCode::Down if key.mods.ctrl => {
            zone.step_slot(false);
            true
        }
        KeyCode::PageUp if key.mods.ctrl => {
            zone.first();
            true
        }
        KeyCode::PageDown if key.mods.ctrl => {
            zone.range.follow();
            true
        }
        KeyCode::PageUp => {
            let rows = usize::from(zone.height);
            zone.scroll(true, rows);
            true
        }
        KeyCode::PageDown => {
            let rows = usize::from(zone.height);
            zone.scroll(false, rows);
            true
        }
        KeyCode::Home if key.mods.ctrl && key.mods.shift => {
            zone.align(false);
            true
        }
        KeyCode::End if key.mods.ctrl && key.mods.shift => {
            zone.align(true);
            true
        }
        _ => false,
    }
}
