// The wordmark drawn in the empty draft: a pixel grid under two half-blocks per cell.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

// The grid the mark was drawn on: `#` a body pixel, `*` the star, `.` nothing.
const GRID: [&str; ROWS] = [
    ".......................................*....",
    "......................................***...",
    "..###..###...###...###...###########...*....",
    ".##########..###...###..#############.......",
    ".###.##.###..###...###..###..###..###.......",
    ".###.##.###..###...###.......###..###.......",
    ".###.##.###..###...###.......###..###.......",
    ".###.##.###..###...###.......###..###.......",
    ".###.##.###..###...###.......###..###.......",
    ".###.##.###..###...###.......###..###.......",
    ".###.##.###...#######........###..###.......",
    ".###.##.###.....#####.......####..####......",
    "...................###......................",
    "...............######.......................",
    "................####........................",
    "............................................",
];

// Rows of pixels; one terminal row holds two of them.
const ROWS: usize = 16;
// Columns of pixels, one terminal column each.
const COLS: usize = 44;
// Rows the mark takes on screen.
const LINES: usize = ROWS / 2;

// The star's own color; every body pixel is on the gradient below.
const STAR: (u8, u8, u8) = (254, 240, 138);

// Where the body's gradient turns over, left to right and bottom to top.
const FROM: (u8, u8, u8) = (244, 114, 182);
const MID: (u8, u8, u8) = (96, 165, 250);
const TO: (u8, u8, u8) = (103, 232, 249);

// The whole screen the draft shows, or nothing when the terminal cannot hold the mark whole.
pub(super) fn screen(width: usize, height: usize) -> Option<Vec<Line<'static>>> {
    if width < COLS || height < LINES {
        return None;
    }
    let left = (width - COLS) / 2;
    let mut out: Vec<Line<'static>> = Vec::with_capacity(height);
    for _ in 0..(height - LINES) / 2 {
        out.push(Line::default());
    }
    for pair in 0..LINES {
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(COLS + 1);
        if left > 0 {
            spans.push(Span::raw(" ".repeat(left)));
        }
        for c in 0..COLS {
            let top = pixel(pair * 2, c);
            let bottom = pixel(pair * 2 + 1, c);
            spans.push(cell(top, bottom));
        }
        out.push(Line::from(spans));
    }
    while out.len() < height {
        out.push(Line::default());
    }
    Some(out)
}

// One column of the grid as one terminal cell: the upper half carries the top pixel in the
// foreground, the lower half the bottom one in the background.
fn cell(top: Option<(u8, u8, u8)>, bottom: Option<(u8, u8, u8)>) -> Span<'static> {
    let color = |(r, g, b): (u8, u8, u8)| Color::Rgb(r, g, b);
    match (top, bottom) {
        (None, None) => Span::raw(" "),
        (Some(top), None) => Span::styled("▀", Style::new().fg(color(top))),
        (None, Some(bottom)) => Span::styled("▄", Style::new().fg(color(bottom))),
        (Some(top), Some(bottom)) if top == bottom => {
            Span::styled("█", Style::new().fg(color(top)))
        }
        (Some(top), Some(bottom)) => {
            Span::styled("▀", Style::new().fg(color(top)).bg(color(bottom)))
        }
    }
}

// One pixel's color, or none where the grid is transparent.
fn pixel(r: usize, c: usize) -> Option<(u8, u8, u8)> {
    let at = GRID[r].as_bytes()[c];
    match at {
        b'#' => Some(gradient(r, c)),
        b'*' => Some(STAR),
        _ => None,
    }
}

// The body's three-stop gradient, running up and to the right.
fn gradient(r: usize, c: usize) -> (u8, u8, u8) {
    let x = c as f32 / (COLS - 1) as f32;
    let y = 1.0 - (r as f32 / (ROWS - 1) as f32);
    let t = (x * 0.72 + y * 0.28).clamp(0.0, 1.0);
    if t < 0.5 {
        mix(FROM, MID, t * 2.0)
    } else {
        mix(MID, TO, (t - 0.5) * 2.0)
    }
}

fn mix(from: (u8, u8, u8), to: (u8, u8, u8), k: f32) -> (u8, u8, u8) {
    let one = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * k) as u8;
    (one(from.0, to.0), one(from.1, to.1), one(from.2, to.2))
}
