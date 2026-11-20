//! Key and mouse capture: crossterm's events become [`Action`].

use ratatui::crossterm::event::{
    KeyCode as CtCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};

/// One user action: which key, what was pasted, which way the wheel turned.
///
/// Carries no meaning. What a key does is decided by whoever claims it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Key(Key),
    Paste(String),
    /// A wheel notch upward. `shift` = one round instead of one slot.
    ScrollUp {
        shift: bool,
    },
    /// A wheel notch downward. `shift` = one round instead of one slot.
    ScrollDown {
        shift: bool,
    },
}

/// A key's identity: the key plus the modifiers held. No press/release state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub code: KeyCode,
    pub mods: Mods,
}

/// The keys this front end knows. Anything else is [`KeyCode::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
    Char(char),
    Enter,
    Esc,
    Tab,
    Backspace,
    Delete,
    Insert,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    F(u8),
    Other,
}

/// Held modifiers. Three flags; an unrecognized modifier is dropped.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mods {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

/// Project a crossterm key. `None` = a release, which carries no action.
pub fn from_key(key: KeyEvent) -> Option<Action> {
    if key.kind == KeyEventKind::Release {
        return None;
    }
    let (code, back) = match key.code {
        CtCode::Char(c) => (KeyCode::Char(c), false),
        CtCode::Enter => (KeyCode::Enter, false),
        CtCode::Esc => (KeyCode::Esc, false),
        CtCode::Tab => (KeyCode::Tab, false),
        CtCode::BackTab => (KeyCode::Tab, true),
        CtCode::Backspace => (KeyCode::Backspace, false),
        CtCode::Delete => (KeyCode::Delete, false),
        CtCode::Insert => (KeyCode::Insert, false),
        CtCode::Up => (KeyCode::Up, false),
        CtCode::Down => (KeyCode::Down, false),
        CtCode::Left => (KeyCode::Left, false),
        CtCode::Right => (KeyCode::Right, false),
        CtCode::Home => (KeyCode::Home, false),
        CtCode::End => (KeyCode::End, false),
        CtCode::PageUp => (KeyCode::PageUp, false),
        CtCode::PageDown => (KeyCode::PageDown, false),
        CtCode::F(n) => (KeyCode::F(n), false),
        _ => (KeyCode::Other, false),
    };
    let mods = Mods {
        ctrl: key.modifiers.contains(KeyModifiers::CONTROL),
        alt: key.modifiers.contains(KeyModifiers::ALT),
        shift: key.modifiers.contains(KeyModifiers::SHIFT) || back,
    };
    Some(Action::Key(Key { code, mods }))
}

/// Bracketed paste arrives as one action, however many lines it carries.
pub fn from_paste(text: String) -> Action {
    Action::Paste(text)
}

/// Only the wheel; every other mouse input is dropped.
pub fn from_mouse(m: &MouseEvent) -> Option<Action> {
    let shift = m.modifiers.contains(KeyModifiers::SHIFT);
    match m.kind {
        MouseEventKind::ScrollUp => Some(Action::ScrollUp { shift }),
        MouseEventKind::ScrollDown => Some(Action::ScrollDown { shift }),
        _ => None,
    }
}
