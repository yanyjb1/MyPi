//! Readable text on top of an arbitrary fill.

use ratatui::style::Color;

/// Rec. 601 luma of a color; `None` for the terminal default (unknowable).
fn luma_of(c: Color) -> Option<f64> {
    let of = |(r, g, b): (u8, u8, u8)| {
        let (r, g, b) = (r as f64, g as f64, b as f64);
        (0.299 * r + 0.587 * g + 0.114 * b) / 255.0
    };
    Some(match c {
        Color::Rgb(r, g, b) => of((r, g, b)),
        Color::Black => 0.0,
        Color::White | Color::Gray => 1.0,
        Color::DarkGray => 0.25,
        Color::Red => of((205, 0, 0)),
        Color::Green => of((0, 205, 0)),
        Color::Yellow => of((205, 205, 0)),
        Color::Blue => of((0, 0, 238)),
        Color::Magenta => of((205, 0, 205)),
        Color::Cyan => of((0, 205, 205)),
        Color::LightRed => of((255, 95, 95)),
        Color::LightGreen => of((95, 255, 95)),
        Color::LightYellow => of((255, 255, 95)),
        Color::LightBlue => of((95, 95, 255)),
        Color::LightMagenta => of((255, 95, 255)),
        Color::LightCyan => of((95, 255, 255)),
        Color::Indexed(_) | Color::Reset => return None,
    })
}

/// Black on light fills, near-white on dark ones. Terminal default counts as dark.
pub fn contrast_text_on(bg: Color) -> Color {
    match luma_of(bg) {
        Some(l) if l > 0.5 => Color::Black,
        _ => Color::Rgb(0xe5, 0xe5, 0xe7),
    }
}
