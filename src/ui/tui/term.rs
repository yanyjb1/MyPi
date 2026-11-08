//! The screen: the alternate screen, the keyboard flags mypi asks for, and giving it all back.
//!
//! Two flags matter. `DISAMBIGUATE_ESCAPE_CODES` makes the terminal report Esc as `\x1b[27u`, so a
//! bare `\x1b` can only be the start of an escape sequence — the whole of Esc's routing rests on
//! that premise. Focus reporting is what tells the app whether the reader is still looking;
//! nothing else asks for it.

use std::time::Duration;

use anyhow::Result;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::text::Line;

use crate::ui::tui::action::{self, Action};

use crate::ui::tui::zone::TermSize;

/// What the terminal produced on its own.
pub enum Input {
    /// The user did something.
    Action(Action),
    /// The window took (`true`) or lost (`false`) focus.
    Focus(bool),
}

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
        let _ = execute!(std::io::stdout(), EnableFocusChange);
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

    /// Put text on the system clipboard: the terminal carries it (OSC 52).
    pub fn copy(&self, text: &str) -> Result<()> {
        use std::io::Write;
        let mut out = std::io::stdout();
        out.write_all(super::clipboard::osc52(text).as_bytes())?;
        out.flush()?;
        Ok(())
    }

    /// Wait up to one frame for terminal input. `None` = nothing worth an action this frame.
    pub fn poll(&self) -> Result<Option<Input>> {
        if !event::poll(Duration::from_millis(16))? {
            return Ok(None);
        }
        match event::read()? {
            event::Event::Key(key) => Ok(action::from_key(key).map(Input::Action)),
            event::Event::Mouse(mouse) => Ok(action::from_mouse(&mouse).map(Input::Action)),
            event::Event::Paste(text) => Ok(Some(Input::Action(action::from_paste(text)))),
            event::Event::FocusGained => Ok(Some(Input::Focus(true))),
            event::Event::FocusLost => Ok(Some(Input::Focus(false))),
            event::Event::Resize(_, _) => Ok(None),
        }
    }

    /// Give the screen back.
    pub fn leave(self) {
        let _ = execute!(std::io::stdout(), DisableFocusChange);
        let _ = execute!(std::io::stdout(), PopKeyboardEnhancementFlags);
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
        let _ = execute!(std::io::stdout(), DisableBracketedPaste);
        ratatui::restore();
    }
}
