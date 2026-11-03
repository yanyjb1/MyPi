//! The one theme color: picked at startup, changed on rename.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

/// A pool wide enough to notice, narrow enough that every entry works on a dark terminal.
const POOL: &[(u8, u8, u8)] = &[
    (0x00, 0xb4, 0xff),
    (0x00, 0xff, 0x88),
    (0xd4, 0xc0, 0x90),
    (0xff, 0xb3, 0x47),
    (0xb3, 0x88, 0xff),
    (0xff, 0x79, 0xc6),
    (0x8b, 0xe9, 0xfd),
    (0xa3, 0xe6, 0x35),
];

/// The live accent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Accent {
    color: Color,
}

impl Accent {
    pub const fn of(color: Color) -> Accent {
        Accent { color }
    }

    /// One entry of the pool, chosen per process.
    pub fn pick() -> Accent {
        use std::hash::{BuildHasher, Hasher};
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u8(1);
        let (r, g, b) = POOL[(h.finish() as usize) % POOL.len()];
        Accent::of(Color::Rgb(r, g, b))
    }

    /// A different entry of the pool, for a rename.
    pub fn pick_other(self) -> Accent {
        let mut next = Accent::pick();
        while next == self && POOL.len() > 1 {
            next = Accent::pick();
        }
        next
    }

    /// `#rrggbb`, the form the app broadcasts.
    pub fn hex(self) -> String {
        match self.color {
            Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
            _ => "#00b4ff".to_string(),
        }
    }

    /// Read one back from the frame that carries it.
    pub fn parse(spec: &str) -> Option<Accent> {
        let hex = spec.strip_prefix('#')?;
        if hex.len() != 6 {
            return None;
        }
        let byte = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
        Some(Accent::of(Color::Rgb(byte(0)?, byte(2)?, byte(4)?)))
    }

    pub fn color(self) -> Color {
        self.color
    }

    pub fn span(self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new().fg(self.color))
    }

    pub fn style(self) -> Style {
        Style::new().fg(self.color)
    }

    pub fn on(self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new().bg(self.color))
    }
}

impl Default for Accent {
    fn default() -> Self {
        Accent::of(Color::Rgb(0x00, 0xb4, 0xff))
    }
}
