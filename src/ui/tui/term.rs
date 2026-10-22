//! The screen: the alternate screen, the keyboard flags mypi asks for, and giving it all back.
//!
//! One flag only. `DISAMBIGUATE_ESCAPE_CODES` makes the terminal report Esc as `\x1b[27u`, so a
//! bare `\x1b` can only be the start of an escape sequence — the whole of Esc's routing rests on
//! that premise. The other two are not asked for: `REPORT_ALL_KEYS_AS_ESCAPE_CODES` spreads the
//! parsing surface over the whole keyboard and nothing reads the result (`Modifier(LeftControl)`
//! noise is its only visible effect), and mypi never reads an alternate key.

use anyhow::Result;
use ratatui::crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::text::Line;

use crate::ui::tui::zone::TermSize;

/// The terminal, taken for as long as this value lives.
pub struct Term {
    inner: ratatui::DefaultTerminal,
}

impl Term {
    /// Take the screen.
    pub fn enter() -> Result<Term> {
        let inner = ratatui::init();
        let _ = execute!(
            std::io::stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
        let _ = execute!(std::io::stdout(), EnableBracketedPaste);
        let _ = execute!(std::io::stdout(), EnableMouseCapture);
        Ok(Term { inner })
    }

    /// How big the terminal is right now.
    pub fn size(&self) -> Result<TermSize> {
        let size = self.inner.size()?;
        Ok(TermSize {
            cols: size.width,
            rows: size.height,
        })
    }

    /// One frame: the owning zone's lines, and the hardware cursor when the editor asks for it.
    ///
    /// ratatui hides the cursor every frame unless the frame says where it goes, so a `None` here
    /// is "no cursor this frame", not "leave it where it was".
    pub fn draw(&mut self, lines: Vec<Line<'static>>, cursor: Option<(u16, u16)>) -> Result<()> {
        self.inner.draw(|f| {
            f.render_widget(ratatui::widgets::Paragraph::new(lines), f.area());
            if let Some((x, y)) = cursor {
                f.set_cursor_position((x, y));
            }
        })?;
        Ok(())
    }

    /// Put text on the system clipboard (OSC 52). A terminal that ignores it stays silent.
    pub fn copy(&mut self, text: &str) -> Result<()> {
        use std::io::Write;
        let mut out = std::io::stdout();
        write!(out, "\x1b]52;c;{}\x07", base64(text.as_bytes()))?;
        out.flush()?;
        Ok(())
    }

    /// Give the screen back.
    pub fn leave(self) {
        let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
        let _ = execute!(std::io::stdout(), DisableBracketedPaste);
        ratatui::restore();
    }
}

/// Base64, no padding-free shortcuts: OSC 52 wants the standard alphabet.
fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}
