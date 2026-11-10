//! The label's sweep: a six-column band of the accent, moving with the stream.

use ratatui::style::Modifier;

use super::super::super::row::{Ink, Run};
use crate::ui::tui::text::char_width;

/// Columns the band takes.
const BAND: usize = 6;

/// The label split into runs, shaded by how close each column sits to the band's heart.
pub(crate) fn runs(text: &str, phase: usize) -> Vec<Run> {
    let total = text.chars().map(char_width).sum::<usize>() + BAND;
    let mut groups: Vec<(Ink, String)> = Vec::new();
    let mut column = 0usize;
    for ch in text.chars() {
        let ink = ink_at((phase + column) % total);
        column += char_width(ch);
        match groups.last_mut() {
            Some((last, buffer)) if *last == ink => buffer.push(ch),
            _ => groups.push((ink, ch.to_string())),
        }
    }
    groups
        .into_iter()
        .map(|(ink, text)| Run::with(text, Some(ink), Modifier::empty()))
        .collect()
}

/// The band's heart is the plain accent, each side one step toward the card fill.
fn ink_at(position: usize) -> Ink {
    match position {
        2 | 3 => Ink::Accent,
        1 | 4 => Ink::AccentShade(1),
        _ => Ink::AccentShade(2),
    }
}
