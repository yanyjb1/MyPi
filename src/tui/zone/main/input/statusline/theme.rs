//! Statusline theme surface — the single place a statusline component gets colors.
//!
//! Why not `Palette`: that is a four-slot legacy façade (`accent` / `gold` /
//! `black` / `muted`) over a ~60-token theme. The statusline needs more slots
//! than four, and every component must be able to *declare* which tokens it
//! consumes. So the engine resolves one `StatusTheme` snapshot per frame and
//! hands it down; a component never reaches for the global theme itself. A
//! mid-session theme switch therefore restyles the row with zero plumbing —
//! the snapshot is rebuilt on the next frame (same contract as `Palette::current`).
//!
//! **Wiring status.** Every slot below resolves to *today's* color. The theme's
//! `statusLine*` tokens are declared and load fine but are not yet
//! authoritative (that audit is deliberately deferred); each slot names the
//! token it will be plugged into, so activating one is a one-line change here
//! and touches no component.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::tui::theme::{theme, ColorToken};

/// Every color a statusline component may ask for.
///
/// One entry per *distinct* rendering decision in the row, not one per theme
/// token. This is also the audit surface: `StatusLine::declared_tokens` unions
/// the components' `ColorPolicy`, and a test asserts the union stays a subset
/// of [`ALL_TOKENS`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    /// Frames, the gauge's arrow and used part, the model pill, the session
    /// pill, the ` | ` / `< ` connectors. (theme: `accent`)
    Accent,
    /// Money and the `?N` / `+N` counts. (theme: `warning`)
    Gold,
    /// The idle `π`. (theme: `muted`)
    Muted,
    /// The capsule band the left group and the session pill sit on.
    /// (theme: `userMessageBg` — see the note in [`StatusTheme::resolve`])
    Capsule,
    /// Plain foreground text: the working-directory path.
    /// (theme: `statusLinePath`; today hardcoded white)
    Text,
    /// Clean git branch. (theme: `statusLineGitClean`)
    GitClean,
    /// Dirty git branch. (theme: `statusLineGitDirty`)
    GitDirty,
    /// The ` > ` connector between left components.
    /// (theme: `statusLineSep`; today the terminal's default foreground)
    Sep,
}

/// Every token, for the coverage audit.
pub const ALL_TOKENS: [Token; 8] = [
    Token::Accent,
    Token::Gold,
    Token::Muted,
    Token::Capsule,
    Token::Text,
    Token::GitClean,
    Token::GitDirty,
    Token::Sep,
];

impl Token {
    const fn idx(self) -> usize {
        match self {
            Token::Accent => 0,
            Token::Gold => 1,
            Token::Muted => 2,
            Token::Capsule => 3,
            Token::Text => 4,
            Token::GitClean => 5,
            Token::GitDirty => 6,
            Token::Sep => 7,
        }
    }
}

/// How a component gets its colors. **Declared, never inferred**: the engine
/// only audits it ([`crate::tui::zone::main::input::statusline::StatusLine::declared_tokens`]), so
/// a component stays free to apply the tokens it names however it needs — the
/// usage gauge, for instance, interleaves accent and unstyled dashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorPolicy {
    /// Colors come from the global theme: these are the tokens the component reads.
    Tokens(&'static [Token]),
    /// The component paints fixed colors and ignores the theme entirely.
    Fixed,
    /// The component builds its own styles (its own color control) and may
    /// still read resolved theme values.
    Custom,
}

/// A per-frame snapshot of the colors the statusline is allowed to use.
#[derive(Debug, Clone, Copy)]
pub struct StatusTheme {
    colors: [Color; ALL_TOKENS.len()],
}

impl StatusTheme {
    /// Snapshot the active theme. Called once per frame by the render loop
    /// (next to `Palette::current`), so a runtime theme switch lands on the
    /// next frame.
    pub fn resolve() -> Self {
        let t = theme();
        let mut colors = [Color::Reset; ALL_TOKENS.len()];
        let mut put = |tok: Token, c: Color| colors[tok.idx()] = c;
        put(Token::Accent, t.color(ColorToken::Accent));
        put(Token::Gold, t.color(ColorToken::Warning));
        put(Token::Muted, t.color(ColorToken::Muted));
        // `statusLineBg` is deliberately not read: the capsule band has always
        // been `userMessageBg` here, while models.yml's `theme.black` override
        // is written into `statusLineBg` (so it currently does nothing). Left
        // as is — which side moves is the token audit's call, not this
        // refactor's.
        put(Token::Capsule, t.color(ColorToken::UserMessageBg));
        put(Token::Text, Color::White);
        put(Token::GitClean, t.color(ColorToken::StatusLineGitClean));
        put(Token::GitDirty, t.color(ColorToken::StatusLineGitDirty));
        put(Token::Sep, Color::Reset);
        Self { colors }
    }

    /// The resolved value of one token.
    pub fn get(&self, tok: Token) -> Color {
        self.colors[tok.idx()]
    }

    /// Build a snapshot from raw values. Test-only: it exists so a test can
    /// prove theme-independence without mutating the process-global theme.
    #[cfg(test)]
    pub(crate) fn from_parts(colors: [Color; ALL_TOKENS.len()]) -> Self {
        Self { colors }
    }

    // ---- span builders: the only place a statusline span is styled ----

    /// Capsule body: the capsule background plus `fg`.
    pub fn capsule(&self, text: impl Into<String>, fg: Color) -> Span<'static> {
        Span::styled(
            text.into(),
            Style::new().bg(self.get(Token::Capsule)).fg(fg),
        )
    }

    /// Capsule background only — the capsule's breathing room.
    pub fn on_capsule(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new().bg(self.get(Token::Capsule)))
    }

    /// A left-group connector: capsule background, separator foreground.
    pub fn sep(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(
            text.into(),
            Style::new()
                .bg(self.get(Token::Capsule))
                .fg(self.get(Token::Sep)),
        )
    }

    /// Foreground only, transparent background.
    pub fn fg(&self, text: impl Into<String>, tok: Token) -> Span<'static> {
        Span::styled(text.into(), Style::new().fg(self.get(tok)))
    }

    /// No style at all: default foreground, transparent background.
    pub fn plain(&self, text: impl Into<String>) -> Span<'static> {
        Span::styled(text.into(), Style::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_token_resolves_and_is_distinct_in_index() {
        let t = StatusTheme::resolve();
        for tok in ALL_TOKENS {
            let _ = t.get(tok);
        }
        let mut seen: Vec<usize> = ALL_TOKENS.iter().map(|t| t.idx()).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen.len(), ALL_TOKENS.len(), "token indices must be unique");
    }
}
