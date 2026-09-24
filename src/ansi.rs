//! ANSI escape-sequence handling — a leaf utility, no dependencies.
//!
//! Lives outside `tui` on purpose: the tool layer strips escape codes from
//! captured process output *before* the text goes anywhere (to the model and
//! into a card), so `agent` needs it too. Putting it in `tui` made the agent
//! layer depend on the renderer, which is backwards.

/// Strip ANSI escape sequences from terminal-captured text.
///
/// Tool output is written to the terminal **verbatim**, so a program that
/// colourises its output (`ls --color=always`, `grep --color=always`, `rg`
/// on a TTY, a progress bar) hands us real escape bytes. Drawn inside a
/// card those bytes are re-interpreted by the terminal: an embedded
/// `ESC[0m` resets the background mid-row, which is what punched holes of
/// default terminal background through the card's black.
///
/// Our own styling owns the appearance; captured escape codes are dropped.
/// Handles CSI (with intermediates), OSC (BEL- or ST-terminated), and the
/// two-byte escape forms; a lone ESC at the end is discarded.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            out.push(c);
            continue;
        }
        match chars.peek().copied() {
            // CSI: ESC [ params intermediates final-byte(@..~)
            Some('[') => {
                chars.next();
                for n in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&n) {
                        break;
                    }
                }
            }
            // OSC: ESC ] ... (BEL | ESC \)
            Some(']') => {
                chars.next();
                while let Some(n) = chars.next() {
                    if n == '\u{7}' {
                        break;
                    }
                    if n == '\u{1b}' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            // Escape with intermediates: ESC + (0x20..=0x2f)* + one final byte
            // (e.g. `ESC ( B`, the charset designation).
            Some(n) if ('\u{20}'..='\u{2f}').contains(&n) => {
                for m in chars.by_ref() {
                    if !('\u{20}'..='\u{2f}').contains(&m) {
                        break; // the final byte
                    }
                }
            }
            // Two-byte escape (ESC followed by a single final byte).
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_csi_and_keeps_the_text() {
        // `ls --color=always` style: colour, then a reset.
        let raw = "\u{1b}[0m\u{1b}[01;34mdir\u{1b}[0m";
        assert_eq!(strip_ansi(raw), "dir");
        // `grep --color=always` also emits EL (erase-line).
        let raw = "\u{1b}[01;31m\u{1b}[Khello\u{1b}[m\u{1b}[K";
        assert_eq!(strip_ansi(raw), "hello");
    }

    #[test]
    fn strips_osc_and_two_byte_escapes() {
        // OSC with BEL terminator (terminal title, hyperlinks)
        assert_eq!(strip_ansi("\u{1b}]0;title\u{7}after"), "after");
        // OSC terminated by ST
        assert_eq!(strip_ansi("\u{1b}]8;;http://x\u{1b}\\link"), "link");
        // Bare two-byte escape
        assert_eq!(strip_ansi("a\u{1b}(Bb"), "ab");
    }

    #[test]
    fn plain_text_is_untouched() {
        assert_eq!(strip_ansi("普通文本 ok"), "普通文本 ok");
        // A lone trailing ESC must not panic or leak.
        assert_eq!(strip_ansi("x\u{1b}"), "x");
    }
}
