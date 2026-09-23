//! Theme colors — injected from the theme section of models.yml; no
//! hardcoded colors in this layer.
//!
//! All UI color lookups go through `Palette`; business code never writes
//! literals like `Color::Green`.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;

use crate::ai::config::Config;

/// A color scheme. Fields are semantic slots, decoupled from concrete values.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    /// Accent: borders, model name, used ctx.
    pub accent: Color,
    /// Money color (gold/yellow).
    pub gold: Color,
    /// Capsule background. Defaults to truecolor black — ANSI indexed black looks gray in Konsole.
    pub black: Color,
    /// The only permitted gray: the π symbol and secondary text.
    pub muted: Color,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            accent: Color::Green,
            gold: Color::Yellow,
            black: Color::Rgb(0, 0, 0),
            muted: Color::DarkGray,
        }
    }
}

impl Palette {
    /// Build from configuration.
    pub fn from_config(cfg: &Config) -> Self {
        Self {
            accent: cfg.theme.accent.to_color(),
            gold: cfg.theme.gold.to_color(),
            black: cfg.theme.black.to_color(),
            muted: cfg.theme.muted.to_color(),
        }
    }

    // ---- span builders: the single place that decides which color applies ----

    /// Accent text, transparent background.
    pub fn accent(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new().fg(self.accent))
    }

    /// Money-colored text, transparent background.
    pub fn gold(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new().fg(self.gold))
    }

    /// Capsule body (black background + given foreground).
    pub fn pill(&self, text: impl Into<String>, fg: Color) -> Span<'static> {
        Span::styled(text.into(), Style::new().bg(self.black).fg(fg))
    }

    /// Black background, default foreground (breathing room inside capsules).
    pub fn on_black(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new().bg(self.black))
    }

    /// Black background + accent foreground (`[M]` / `[D]` style markers).
    pub fn mark(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new().bg(self.black).fg(self.accent))
    }

    /// Black background + money color.
    pub fn gold_on_black(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new().bg(self.black).fg(self.gold))
    }

    /// Transparent background, default foreground (placeholder dashes).
    pub fn plain(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new())
    }

    /// Muted gray text, transparent background.
    pub fn muted(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new().fg(self.muted))
    }

    /// Muted gray + italic: helper text such as completion descriptions and thinking blocks.
    pub fn muted_italic(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(
            text.into(),
            Style::new().fg(self.muted).add_modifier(Modifier::ITALIC),
        )
    }

    /// First character: π when idle (gray), a spinner frame while waiting (accent).
    pub fn symbol(&self, spinner: Option<char>) -> Span<'static> {
        match spinner {
            Some(c) => Span::styled(c.to_string(), Style::new().bg(self.black).fg(self.accent)),
            None => Span::styled("π", Style::new().bg(self.black).fg(self.muted)),
        }
    }
}
