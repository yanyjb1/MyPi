//! Key to intent: the input zone's own reading of a key press. One key, one meaning.
//!
//! A key that is lent away never reaches this table; the borrower reads it.

use crate::ui::tui::action::{Key, KeyCode, Mods};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    // Insert one character.
    Insert(char),
    // Paste a block of text from the clipboard or the terminal.
    Paste(String),
    // Insert a newline at the cursor.
    Newline,
    // Delete one character before the cursor.
    Backspace,
    // Delete the character at the cursor.
    Delete,
    Left,
    Right,
    // Move the cursor up one visual row.
    Up,
    // Move the cursor down one visual row.
    Down,
    // Move to the logical line start / end.
    LineHome,
    LineEnd,
    // Move to the start / end of the text.
    DocHome,
    DocEnd,
    // Jump one word left / right by separators.
    WordLeft,
    WordRight,
    // Delete one word before the cursor (Ctrl+W, Ctrl+H, Alt+Backspace).
    DeleteWordBackward,
    // Delete one word after the cursor (Alt+D).
    DeleteWordForward,
    // Delete to the logical line start / end.
    DeleteToLineStart,
    DeleteToLineEnd,
    // Undo / redo.
    Undo,
    Redo,
    // Toggle reasoning fold / tool output fold.
    ToggleReasoning,
    ToggleTools,
    // Clear the input (Ctrl+C).
    ClearInput,
    // Interrupt the in-flight round (Esc, not lent away).
    Interrupt,
    // Submit the current input.
    Submit,
    // Unrecognized; ignore.
    None,
}

pub fn translate(key: Key) -> Intent {
    let Key { code, mods } = key;
    let Mods { ctrl, alt, shift } = mods;

    if ctrl && matches!(code, KeyCode::Char('c')) {
        return Intent::ClearInput;
    }
    if ctrl && matches!(code, KeyCode::Char('z')) {
        return if shift { Intent::Redo } else { Intent::Undo };
    }
    match code {
        KeyCode::Enter => {
            if alt || shift {
                Intent::Newline
            } else {
                Intent::Submit
            }
        }
        // Ctrl+J: the newline fallback (Alt+Enter is unreliable in some terminals).
        KeyCode::Char('j') if ctrl => Intent::Newline,
        KeyCode::Backspace => {
            if ctrl || alt {
                Intent::DeleteWordBackward
            } else {
                Intent::Backspace
            }
        }
        KeyCode::Delete => {
            if ctrl || alt {
                Intent::DeleteWordForward
            } else {
                Intent::Delete
            }
        }
        KeyCode::Char('d') if ctrl => Intent::Delete,
        KeyCode::Char('w') if ctrl => Intent::DeleteWordBackward,
        // Most terminals send Ctrl+Backspace as 0x08, which reads as Ctrl+H.
        KeyCode::Char('h') if ctrl => Intent::DeleteWordBackward,
        KeyCode::Char('d') if alt => Intent::DeleteWordForward,
        KeyCode::Char('u') if ctrl => Intent::DeleteToLineStart,
        KeyCode::Char('k') if ctrl => Intent::DeleteToLineEnd,
        KeyCode::Char('t') if ctrl => Intent::ToggleReasoning,
        KeyCode::Char('o') if ctrl => Intent::ToggleTools,
        KeyCode::Left => {
            if ctrl {
                Intent::WordLeft
            } else {
                Intent::Left
            }
        }
        KeyCode::Right => {
            if ctrl {
                Intent::WordRight
            } else {
                Intent::Right
            }
        }
        // Ctrl+arrows belong to the history zone.
        KeyCode::Up => {
            if ctrl {
                Intent::None
            } else {
                Intent::Up
            }
        }
        KeyCode::Down => {
            if ctrl {
                Intent::None
            } else {
                Intent::Down
            }
        }
        KeyCode::Home => {
            if ctrl {
                Intent::DocHome
            } else {
                Intent::LineHome
            }
        }
        KeyCode::End => {
            if ctrl {
                Intent::DocEnd
            } else {
                Intent::LineEnd
            }
        }
        // Plain characters, including those committed by CJK IMEs.
        KeyCode::Char(c) => {
            if ctrl {
                // Ctrl+V: with bracketed paste the terminal sends the block itself.
                if c == 'v' {
                    return Intent::Paste(String::new());
                }
                Intent::None
            } else {
                Intent::Insert(c)
            }
        }
        _ => Intent::None,
    }
}
