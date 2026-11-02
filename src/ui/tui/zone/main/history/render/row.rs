//! One row before its colors: text runs tagged with a token and modifiers.
//!
//! Layout produces rows; colors are applied when a row is painted. Nothing here knows a
//! theme, so a layout survives a theme change and only the visit to the screen costs color.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::ui::tui::theme::{HistoryTheme, Token};

/// One run of text and how it is colored.
#[derive(Debug, Clone, PartialEq)]
pub struct Piece {
    pub text: String,
    /// `None` = the terminal's own foreground.
    pub token: Option<Token>,
    pub attrs: Modifier,
}

impl Piece {
    pub fn token(text: impl Into<String>, token: Token) -> Piece {
        Piece {
            text: text.into(),
            token: Some(token),
            attrs: Modifier::empty(),
        }
    }

    pub fn raw(text: impl Into<String>) -> Piece {
        Piece {
            text: text.into(),
            token: None,
            attrs: Modifier::empty(),
        }
    }

    pub fn with(text: impl Into<String>, token: Option<Token>, attrs: Modifier) -> Piece {
        Piece {
            text: text.into(),
            token,
            attrs,
        }
    }
}

/// One laid-out row. `Done` carries its colors: cards and tables resolve them while drawing
/// their own frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Laid {
    Pieces(Vec<Piece>),
    Done(Line<'static>),
}

impl Laid {
    pub fn empty() -> Laid {
        Laid::Pieces(Vec::new())
    }

    pub fn is_empty(&self) -> bool {
        match self {
            Laid::Pieces(pieces) => pieces.iter().all(|piece| piece.text.is_empty()),
            Laid::Done(line) => line.spans.iter().all(|span| span.content.is_empty()),
        }
    }

    /// This row in the theme's colors.
    pub fn color(&self, t: &HistoryTheme) -> Line<'static> {
        match self {
            Laid::Done(line) => line.clone(),
            Laid::Pieces(pieces) => {
                let spans: Vec<Span<'static>> = pieces
                    .iter()
                    .map(|piece| {
                        let mut style = Style::new().add_modifier(piece.attrs);
                        if let Some(token) = piece.token {
                            style = style.fg(t.get(token));
                        }
                        Span::styled(piece.text.clone(), style)
                    })
                    .collect();
                Line::from(spans)
            }
        }
    }
}
