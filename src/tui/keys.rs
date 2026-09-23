//! Key mapping — crossterm `KeyEvent` -> semantic action.
//!
//! Why a separate layer: it splits which key does what from how it is implemented.
//! Rebinding, adding keys, or building a configurable keymap later only touches this file.
//!
//! One real terminal limitation: **many terminals cannot send an encoding for
//! Ctrl+Shift+letter distinct from Ctrl+letter**. Under Konsole, Ctrl+V and
//! Ctrl+Shift+V can both arrive as `Char('v') + CONTROL`. Paste therefore cannot
//! rely on Ctrl+V alone — bracketed paste (see `app.rs`) is the reliable channel; Ctrl+V only asks the terminal to paste as a fallback.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

// Editor semantic actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    // Insert one character.
    Insert(char),
    // Paste a whole block of text from the clipboard / terminal (possibly multiline).
    Paste(String),
    // Insert a newline at the cursor.
    Newline,
    // Delete one character before the cursor.
    Backspace,
    // Delete the character at the cursor.
    Delete,
    // Move the cursor left one step.
    Left,
    // Move the cursor right one step.
    Right,
    // Move the cursor up one visual row.
    Up,
    // Move the cursor down one visual row.
    Down,
    // Move to the logical line start.
    LineHome,
    // Move to the logical line end.
    LineEnd,
    // Move to the start of the text.
    DocHome,
    // Move to the end of the text.
    DocEnd,
    // Jump one word left by separators.
    WordLeft,
    // Jump one word right by separators.
    WordRight,
    // Delete one word before the cursor (Ctrl+W).
    DeleteWordBackward,
    // Delete one word after the cursor (Alt+D).
    DeleteWordForward,
    // Delete to the logical line start (Ctrl+U).
    DeleteToLineStart,
    // Delete to the logical line end (Ctrl+K).
    DeleteToLineEnd,
    // Undo (Ctrl+Z).
    Undo,
    // Redo.
    Redo,
    // Previous input history entry (↑ on the first row).
    HistoryPrev,
    // Next input history entry (↓ on the last row).
    HistoryNext,
    // /resume picker: ↑↓ move.
    SelectorUp,
    SelectorDown,
    // Enter confirms restoring the highlighted session.
    SelectorConfirm,
    // Esc closes the picker (no restore).
    SelectorCancel,
    // Trigger / advance path completion (Tab).
    Complete,
    // Move up in the completion popup.
    CompleteUp,
    // Move down in the completion popup.
    CompleteDown,
    // Close the completion popup (Esc).
    DismissCompletion,
    // Toggle reasoning fold (Ctrl+T).
    ToggleReasoning,
    // Toggle tool-output expansion (Ctrl+O).
    ToggleTools,
    // Clear the input (Ctrl+C).
    ClearInput,
    // Interrupt the in-flight streaming reply (Esc).
    Interrupt,
    // Submit the current input.
    Submit,
    // Quit the program.
    Quit,
    // Tree navigator modal: move highlight / confirm / cancel.
    TreeUp,
    TreeDown,
    TreeConfirm,
    TreeCancel,
    // Unrecognized; ignore.
    None,
}

// Context required to translate keys.
//
// Why these are passed in: the meaning of `Esc` / `Ctrl+C` / `↑` / `↓` **depends on current state**.
// Esc closes the completion popup when it is open, interrupts during streaming,
// and only otherwise quits. Passing state explicitly keeps `translate` a pure function that unit-tests easily.
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyContext {
    // The input box is empty.
    pub editor_empty: bool,
    // Waiting for the model reply.
    pub streaming: bool,
    // The completion popup is open.
    pub popup_open: bool,
    // The /resume picker is open (modal: ↑↓/Enter/Esc are all taken over).
    pub selector_open: bool,
    // The tree navigator modal is open (full-screen takeover).
    pub tree_open: bool,
    // The cursor is on the first visual row.
    pub at_first_line: bool,
    // The cursor is on the last visual row.
    pub at_last_line: bool,
    // Currently browsing input history.
    pub browsing_history: bool,
}

// Translate a key into an action.
//
// Keybindings align with pi (`packages/tui/src/keybindings.ts` +
// `core/keybindings.ts`), so muscle memory from pi transfers directly.
pub fn translate(key: KeyEvent) -> Action {
    translate_with(key, KeyContext::default())
}

// Context-aware translation. Same logic as `translate`, plus state checks.
pub fn translate_with(key: KeyEvent, cx: KeyContext) -> Action {
    let m = key.modifiers;
    let ctrl = m.contains(KeyModifiers::CONTROL);
    let alt = m.contains(KeyModifiers::ALT);
    let shift = m.contains(KeyModifiers::SHIFT);

    // ---- tree navigator modal: full-screen, highest priority ----
    if cx.tree_open {
        match key.code {
            KeyCode::Up => return Action::TreeUp,
            KeyCode::Down => return Action::TreeDown,
            KeyCode::Enter if !alt && !shift => return Action::TreeConfirm,
            KeyCode::Esc => return Action::TreeCancel,
            _ => {}
        }
    }

    // ---- /resume session picker: modal, highest priority ----
    if cx.selector_open {
        match key.code {
            KeyCode::Up => return Action::SelectorUp,
            KeyCode::Down => return Action::SelectorDown,
            KeyCode::Enter if !alt && !shift => return Action::SelectorConfirm,
            KeyCode::Esc => return Action::SelectorCancel,
            _ => {}
        }
    }


    // ---- while the completion popup is open, these keys are taken over first ----
    if cx.popup_open {
        match key.code {
            KeyCode::Up => return Action::CompleteUp,
            KeyCode::Down => return Action::CompleteDown,
            KeyCode::Tab => return Action::Complete,
            KeyCode::Enter if !alt && !shift => return Action::Complete,
            KeyCode::Esc => return Action::DismissCompletion,
            _ => {}
        }
    }

    // ---- Esc: close popup > interrupt reply > quit ----
    if matches!(key.code, KeyCode::Esc) {
        if cx.popup_open {
            return Action::DismissCompletion;
        }
        if cx.streaming {
            return Action::Interrupt;
        }
        return Action::Quit;
    }

    // ---- Ctrl+C: clear input; quit only when the input is already empty ----
    // This follows pi's convention (`app.clear` / `app.exit`) and is far safer than
    // quitting outright: abandoning a half-written message never kills the program.
    if ctrl && matches!(key.code, KeyCode::Char('c')) {
        return if cx.editor_empty {
            Action::Quit
        } else {
            Action::ClearInput
        };
    }

    // ---- Ctrl+D: quit when the editor is empty (pi's `app.exit`) ----
    // When non-empty it deletes forward one char; see the Char('d') arm below.
    if ctrl && matches!(key.code, KeyCode::Char('d')) && cx.editor_empty {
        return Action::Quit;
    }

    // ---- Ctrl+Z / Ctrl+Shift+Z: undo / redo ----
    if ctrl && matches!(key.code, KeyCode::Char('z')) {
        return if shift { Action::Redo } else { Action::Undo };
    }

    match key.code {
        // Enter family
        KeyCode::Enter => {
            if alt || shift {
                Action::Newline
            } else {
                Action::Submit
            }
        }
        // Ctrl+J as the newline fallback (Alt+Enter is unreliable in some terminals)
        KeyCode::Char('j') if ctrl => Action::Newline,

        // ---- deletion ----
        // Backspace: bare deletes one char; with Ctrl/Alt deletes a word.
        // Terminal reality: most terminals send `\x08` for Ctrl+Backspace
        // while plain Backspace sends `\x7f` (see crossterm parsing); both
        // reach here. Some terminals, Konsole included, may deliver
        // Ctrl+Backspace as plain Backspace — it then degrades to deleting
        // one char. Not wrong, just slower.
        KeyCode::Backspace => {
            if ctrl || alt {
                Action::DeleteWordBackward
            } else {
                Action::Backspace
            }
        }
        // Delete: bare deletes forward one char; Ctrl+D is a synonym (pi's binding,
        // vim/readline habit); Ctrl/Alt+Delete deletes a word right.
        KeyCode::Delete => {
            if ctrl || alt {
                Action::DeleteWordForward
            } else {
                Action::Delete
            }
        }
        // Ctrl+D: delete forward one char (matches pi's `deleteCharForward`)
        KeyCode::Char('d') if ctrl => Action::Delete,
        // Ctrl+W (beyond the Alt+Backspace pair): delete a word left (readline habit)
        KeyCode::Char('w') if ctrl => Action::DeleteWordBackward,
        // Ctrl+Backspace: **most terminals encode it as 0x08, and 0x08 is
        // Ctrl+H** (0x08 = 'h' - 'a' + 1; BS and Ctrl+H share a code
        // historically). crossterm therefore reports `Char('h') + CONTROL`,
        // recognizable only here. The cost: a real Ctrl+H also deletes a
        // word — acceptable in a chat input, and in exchange
        // Ctrl+Backspace word deletion works on Konsole and xterm.
        KeyCode::Char('h') if ctrl => Action::DeleteWordBackward,
        // Alt+D: delete a word right (matches pi's `deleteWordForward`)
        KeyCode::Char('d') if alt => Action::DeleteWordForward,
        // Ctrl+U / Ctrl+K: delete to line start / end (matches pi)
        KeyCode::Char('u') if ctrl => Action::DeleteToLineStart,
        KeyCode::Char('k') if ctrl => Action::DeleteToLineEnd,
        // Fold toggles: global view actions, independent of editing state
        KeyCode::Char('t') if ctrl => Action::ToggleReasoning,
        KeyCode::Char('o') if ctrl => Action::ToggleTools,

        // arrow keys
        KeyCode::Left => {
            if ctrl {
                Action::WordLeft
            } else {
                Action::Left
            }
        }
        KeyCode::Right => {
            if ctrl {
                Action::WordRight
            } else {
                Action::Right
            }
        }
        // ↑: switches to history on the first row when (the input is empty or history
        // is being browsed); otherwise ordinary up. Mirrors pi's editor.ts:925 —
        // without the condition, multiline input could never use ↑.
        KeyCode::Up => {
            if cx.at_first_line && (cx.editor_empty || cx.browsing_history) {
                Action::HistoryPrev
            } else {
                Action::Up
            }
        }
        // ↓: on the last row while browsing history, switch to the next entry
        KeyCode::Down => {
            if cx.at_last_line && cx.browsing_history {
                Action::HistoryNext
            } else {
                Action::Down
            }
        }

        // Tab: path completion
        KeyCode::Tab => Action::Complete,

        // Home/End: with Ctrl they act on the whole text, otherwise the current logical line
        KeyCode::Home => {
            if ctrl {
                Action::DocHome
            } else {
                Action::LineHome
            }
        }
        KeyCode::End => {
            if ctrl {
                Action::DocEnd
            } else {
                Action::LineEnd
            }
        }

        // Plain characters (including those committed by CJK IMEs)
        KeyCode::Char(c) => {
            if ctrl {
                // Ctrl+V: terminals with support trigger bracketed paste directly;
                // without it the clipboard content is unavailable and the app layer
                // turns this into an empty paste.
                if c == 'v' {
                    return Action::Paste(String::new());
                }
                Action::None
            } else {
                Action::Insert(c)
            }
        }
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, m: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, m)
    }

    #[test]
    fn enter_submits_alt_enter_newlines() {
        assert_eq!(translate(key(KeyCode::Enter, KeyModifiers::NONE)), Action::Submit);
        assert_eq!(translate(key(KeyCode::Enter, KeyModifiers::ALT)), Action::Newline);
        assert_eq!(translate(key(KeyCode::Enter, KeyModifiers::SHIFT)), Action::Newline);
    }

    #[test]
    fn esc_quits_when_idle() {
        assert_eq!(translate(key(KeyCode::Esc, KeyModifiers::NONE)), Action::Quit);
    }

    #[test]
    fn ctrl_c_clears_input_then_quits_when_empty() {
        // Non-empty: clear (do not quit — abandoning a draft must not kill the program)
        let busy = KeyContext { editor_empty: false, ..Default::default() };
        assert_eq!(
            translate_with(key(KeyCode::Char('c'), KeyModifiers::CONTROL), busy),
            Action::ClearInput
        );
        // Empty: quit
        let empty = KeyContext { editor_empty: true, ..Default::default() };
        assert_eq!(
            translate_with(key(KeyCode::Char('c'), KeyModifiers::CONTROL), empty),
            Action::Quit
        );
    }

    #[test]
    fn esc_interrupts_while_streaming() {
        let cx = KeyContext { streaming: true, editor_empty: false, ..Default::default() };
        assert_eq!(translate_with(key(KeyCode::Esc, KeyModifiers::NONE), cx), Action::Interrupt);
    }

    #[test]
    fn esc_dismisses_popup_first() {
        let cx = KeyContext {
            popup_open: true,
            streaming: true, // the popup wins even while streaming
            ..Default::default()
        };
        assert_eq!(
            translate_with(key(KeyCode::Esc, KeyModifiers::NONE), cx),
            Action::DismissCompletion
        );
    }

    #[test]
    fn ctrl_d_quits_only_when_empty() {
        let empty = KeyContext { editor_empty: true, ..Default::default() };
        assert_eq!(
            translate_with(key(KeyCode::Char('d'), KeyModifiers::CONTROL), empty),
            Action::Quit
        );
        let busy = KeyContext { editor_empty: false, ..Default::default() };
        assert_eq!(
            translate_with(key(KeyCode::Char('d'), KeyModifiers::CONTROL), busy),
            Action::Delete
        );
    }

    #[test]
    fn ctrl_z_undoes_and_shift_redoes() {
        assert_eq!(
            translate(key(KeyCode::Char('z'), KeyModifiers::CONTROL)),
            Action::Undo
        );
        assert_eq!(
            translate(
                key(KeyCode::Char('z'), KeyModifiers::CONTROL | KeyModifiers::SHIFT)
            ),
            Action::Redo
        );
    }

    #[test]
    fn tab_requests_completion() {
        assert_eq!(translate(key(KeyCode::Tab, KeyModifiers::NONE)), Action::Complete);
    }

    #[test]
    fn up_down_switch_to_history_at_edges() {
        // First row + empty input -> browse history
        let top = KeyContext { at_first_line: true, editor_empty: true, ..Default::default() };
        assert_eq!(translate_with(key(KeyCode::Up, KeyModifiers::NONE), top), Action::HistoryPrev);
        // First row but non-empty input -> ordinary up (what makes ↑ usable in multiline input)
        let mid = KeyContext { at_first_line: true, editor_empty: false, ..Default::default() };
        assert_eq!(translate_with(key(KeyCode::Up, KeyModifiers::NONE), mid), Action::Up);
        // While browsing history, ↑ on the first row keeps browsing
        let browsing = KeyContext {
            at_first_line: true,
            editor_empty: false,
            browsing_history: true,
            ..Default::default()
        };
        assert_eq!(
            translate_with(key(KeyCode::Up, KeyModifiers::NONE), browsing),
            Action::HistoryPrev
        );
        // Last row + browsing history -> next entry
        let bottom = KeyContext {
            at_last_line: true,
            browsing_history: true,
            editor_empty: false,
            ..Default::default()
        };
        assert_eq!(
            translate_with(key(KeyCode::Down, KeyModifiers::NONE), bottom),
            Action::HistoryNext
        );
        // Last row but not browsing -> ordinary down
        let plain = KeyContext { at_last_line: true, editor_empty: false, ..Default::default() };
        assert_eq!(translate_with(key(KeyCode::Down, KeyModifiers::NONE), plain), Action::Down);
    }

    #[test]
    fn popup_owns_arrows_and_enter() {
        let cx = KeyContext { popup_open: true, editor_empty: false, ..Default::default() };
        assert_eq!(translate_with(key(KeyCode::Up, KeyModifiers::NONE), cx), Action::CompleteUp);
        assert_eq!(translate_with(key(KeyCode::Down, KeyModifiers::NONE), cx), Action::CompleteDown);
        assert_eq!(translate_with(key(KeyCode::Tab, KeyModifiers::NONE), cx), Action::Complete);
        assert_eq!(translate_with(key(KeyCode::Enter, KeyModifiers::NONE), cx), Action::Complete);
    }

    #[test]
    fn ctrl_arrows_are_word_navigation() {
        assert_eq!(translate(key(KeyCode::Left, KeyModifiers::CONTROL)), Action::WordLeft);
        assert_eq!(translate(key(KeyCode::Right, KeyModifiers::CONTROL)), Action::WordRight);
        assert_eq!(translate(key(KeyCode::Left, KeyModifiers::NONE)), Action::Left);
        assert_eq!(translate(key(KeyCode::Right, KeyModifiers::NONE)), Action::Right);
    }

    #[test]
    fn arrows_are_vertical_navigation() {
        assert_eq!(translate(key(KeyCode::Up, KeyModifiers::NONE)), Action::Up);
        assert_eq!(translate(key(KeyCode::Down, KeyModifiers::NONE)), Action::Down);
    }

    #[test]
    fn home_end_scope_depends_on_ctrl() {
        assert_eq!(translate(key(KeyCode::Home, KeyModifiers::NONE)), Action::LineHome);
        assert_eq!(translate(key(KeyCode::Home, KeyModifiers::CONTROL)), Action::DocHome);
        assert_eq!(translate(key(KeyCode::End, KeyModifiers::NONE)), Action::LineEnd);
        assert_eq!(translate(key(KeyCode::End, KeyModifiers::CONTROL)), Action::DocEnd);
    }

    #[test]
    fn plain_char_inserts_cjk() {
        assert_eq!(translate(key(KeyCode::Char('中'), KeyModifiers::NONE)), Action::Insert('中'));
    }

    #[test]
    fn ctrl_j_newlines() {
        assert_eq!(translate(key(KeyCode::Char('j'), KeyModifiers::CONTROL)), Action::Newline);
    }

    #[test]
    fn ctrl_v_requests_paste() {
        assert_eq!(
            translate(key(KeyCode::Char('v'), KeyModifiers::CONTROL)),
            Action::Paste(String::new())
        );
    }

    // ---- fast-deletion keybindings ----

    #[test]
    fn ctrl_backspace_and_ctrl_w_delete_word_backward() {
        assert_eq!(
            translate(key(KeyCode::Backspace, KeyModifiers::CONTROL)),
            Action::DeleteWordBackward
        );
        assert_eq!(
            translate(key(KeyCode::Backspace, KeyModifiers::ALT)),
            Action::DeleteWordBackward
        );
        assert_eq!(
            translate(key(KeyCode::Char('w'), KeyModifiers::CONTROL)),
            Action::DeleteWordBackward
        );
        // Bare Backspace still deletes exactly one char
        assert_eq!(translate(key(KeyCode::Backspace, KeyModifiers::NONE)), Action::Backspace);
    }

    #[test]
    fn ctrl_d_and_delete_variants() {
        // Ctrl+D deletes forward one char (pi's deleteCharForward)
        assert_eq!(translate(key(KeyCode::Char('d'), KeyModifiers::CONTROL)), Action::Delete);
        // Bare Delete deletes right one char; with modifiers a word
        assert_eq!(translate(key(KeyCode::Delete, KeyModifiers::NONE)), Action::Delete);
        assert_eq!(
            translate(key(KeyCode::Delete, KeyModifiers::CONTROL)),
            Action::DeleteWordForward
        );
        // Alt+D deletes a word right (pi's deleteWordForward)
        assert_eq!(
            translate(key(KeyCode::Char('d'), KeyModifiers::ALT)),
            Action::DeleteWordForward
        );
    }

    #[test]
    fn ctrl_h_is_ctrl_backspace() {
        // Terminals encode Ctrl+Backspace as 0x08, which crossterm parses as Ctrl+H
        assert_eq!(
            translate(key(KeyCode::Char('h'), KeyModifiers::CONTROL)),
            Action::DeleteWordBackward
        );
        // A bare h stays a plain character
        assert_eq!(translate(key(KeyCode::Char('h'), KeyModifiers::NONE)), Action::Insert('h'));
    }

    #[test]
    fn delete_variants_cover_real_terminal_encodings() {
        // The encodings below were verified live in tmux via examples/keyprobe.rs:
        // 0x7f → Backspace；ESC 0x7f → Backspace+ALT
        // CSI 3;5~ → Delete+CONTROL；ESC d → Char('d')+ALT
        assert_eq!(
            translate(key(KeyCode::Backspace, KeyModifiers::ALT)),
            Action::DeleteWordBackward
        );
        assert_eq!(
            translate(key(KeyCode::Delete, KeyModifiers::CONTROL)),
            Action::DeleteWordForward
        );
        assert_eq!(
            translate(key(KeyCode::Char('d'), KeyModifiers::ALT)),
            Action::DeleteWordForward
        );
    }

    #[test]
    fn ctrl_u_k_delete_to_line_bounds() {
        assert_eq!(
            translate(key(KeyCode::Char('u'), KeyModifiers::CONTROL)),
            Action::DeleteToLineStart
        );
        assert_eq!(
            translate(key(KeyCode::Char('k'), KeyModifiers::CONTROL)),
            Action::DeleteToLineEnd
        );
    }

    #[test]
    fn plain_d_still_inserts() {
        assert_eq!(translate(key(KeyCode::Char('d'), KeyModifiers::NONE)), Action::Insert('d'));
        assert_eq!(translate(key(KeyCode::Char('w'), KeyModifiers::NONE)), Action::Insert('w'));
        assert_eq!(translate(key(KeyCode::Char('u'), KeyModifiers::NONE)), Action::Insert('u'));
        assert_eq!(translate(key(KeyCode::Char('k'), KeyModifiers::NONE)), Action::Insert('k'));
    }
}
