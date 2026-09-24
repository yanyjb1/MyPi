//! Color math — the small numeric core the theme system needs.
//!
//! Ported from omp `theme/color.ts` + `pi-utils/color`, keeping only what a
//! TUI theme uses: hex parsing, perceived luma, and the contrast rule
//! (light surface -> dark text, dark surface -> light text).

use ratatui::style::Color;

/// Parse `#rrggbb` (3- or 6-digit) into `(r, g, b)`.
pub fn parse_hex(s: &str) -> Option<(u8, u8, u8)> {
    let hex = s.strip_prefix('#')?;
    match hex.len() {
        6 => Some((
            u8::from_str_radix(&hex[0..2], 16).ok()?,
            u8::from_str_radix(&hex[2..4], 16).ok()?,
            u8::from_str_radix(&hex[4..6], 16).ok()?,
        )),
        _ => None,
    }
}

/// Perceived brightness via Rec. 601 luma, 0.0 (black) ..= 1.0 (white).
///
/// omp uses this to decide whether a background is "light" (luma > 0.5)
/// and to pick readable text on top of an arbitrary fill.
pub fn color_luma(c: (u8, u8, u8)) -> f64 {
    let (r, g, b) = (c.0 as f64, c.1 as f64, c.2 as f64);
    (0.299 * r + 0.587 * g + 0.114 * b) / 255.0
}

/// Luma of a ratatui color; `None` for the terminal default (unknowable).
pub fn luma_of(c: Color) -> Option<f64> {
    match c {
        Color::Rgb(r, g, b) => Some(color_luma((r, g, b))),
        Color::Black => Some(0.0),
        Color::White | Color::Gray => Some(1.0),
        Color::DarkGray => Some(0.25),
        Color::Red => Some(color_luma((205, 0, 0))),
        Color::Green => Some(color_luma((0, 205, 0))),
        Color::Yellow => Some(color_luma((205, 205, 0))),
        Color::Blue => Some(color_luma((0, 0, 238))),
        Color::Magenta => Some(color_luma((205, 0, 205))),
        Color::Cyan => Some(color_luma((0, 205, 205))),
        Color::LightRed => Some(color_luma((255, 95, 95))),
        Color::LightGreen => Some(color_luma((95, 255, 95))),
        Color::LightYellow => Some(color_luma((255, 255, 95))),
        Color::LightBlue => Some(color_luma((95, 95, 255))),
        Color::LightMagenta => Some(color_luma((255, 95, 255))),
        Color::LightCyan => Some(color_luma((95, 255, 255))),
        Color::Indexed(_) | Color::Reset => None,
    }
}

/// The readable text color on top of `bg`: black on light fills, near-white
/// on dark ones. Terminal default (`None`) counts as dark — terminals the
/// user actually pairs with this program are dark.
pub fn contrast_text_on(bg: Color) -> Color {
    match luma_of(bg) {
        Some(l) if l > 0.5 => Color::Black,
        _ => Color::Rgb(0xe5, 0xe5, 0xe7), // omp's light-theme default text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_parsing() {
        assert_eq!(parse_hex("#00b4ff"), Some((0x00, 0xb4, 0xff)));
        assert_eq!(parse_hex("#ABCDEF"), Some((0xab, 0xcd, 0xef)));
        assert_eq!(parse_hex("00b4ff"), None);
        assert_eq!(parse_hex("#00b4"), None);
    }

    #[test]
    fn luma_ordering() {
        assert!(color_luma((0, 0, 0)) < color_luma((128, 128, 128)));
        assert!(color_luma((255, 255, 255)) > 0.99);
    }

    #[test]
    fn contrast_rule() {
        assert_eq!(contrast_text_on(Color::Rgb(240, 240, 240)), Color::Black);
        assert_eq!(
            contrast_text_on(Color::Rgb(10, 10, 10)),
            Color::Rgb(0xe5, 0xe5, 0xe7)
        );
    }
}
