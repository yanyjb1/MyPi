// One line-addressed change: the piece around it, its line numbers, and the one-line receipt.

use serde_json::{Value, json};

use super::lang::Table;
use super::mask;

// Lines a window keeps above and below the change.
const WINDOW: usize = 5;

// The context of one change: `{"excerpt": {"from": n, "lines": [...], "gap": [...]}}`. `at` is the first changed line, 1-based; `span` is how many old lines the change covers. The excerpt is the piece holding the change, or a window around it when no piece holds it.
pub(crate) fn context(table: &Table, text: &str, at: usize, span: usize) -> Value {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return json!({ "excerpt": { "from": 1, "lines": [], "gap": [] } });
    }
    let at = at.clamp(1, lines.len());
    let end = (at + span.max(1) - 1).min(lines.len());
    let (from, to) = match table.innermost(at - 1) {
        // The piece stands in only for a change below its own first line; one on that line draws the window.
        Some((first, line)) if first < at && line.mate.unwrap_or(first) + 1 >= end => {
            (first, line.mate.unwrap_or(first))
        }
        _ => window(at, end, lines.len()),
    };

    // Masked where it is drawn: a payload a batch carried through rides in no answer, and a folded run of it stands as the one line that speaks for all of it, which a piece reaching from a changed line up past a fold would otherwise carry whole.
    let drawn = mask::block(&lines);
    let (from, to) = mask::around(&drawn, from, at - 1, to).unwrap_or((from, from));
    let shown: Vec<String> = drawn[from..=to]
        .iter()
        .filter_map(|held| match held {
            mask::Shown::Line(line) => Some(line.to_string()),
            mask::Shown::Skip => None,
        })
        .collect();
    json!({ "excerpt": { "from": from + 1, "lines": shown, "gap": stood(&drawn, from, to) } })
}

// Where the drawn lines of a range stand, 1-based, so a card that draws them numbers each by its own place in the file rather than by its turn in the list.
fn stood(drawn: &[mask::Shown<'_>], from: usize, to: usize) -> Vec<usize> {
    drawn[from..=to]
        .iter()
        .enumerate()
        .filter(|(_, held)| matches!(held, mask::Shown::Line(_)))
        .map(|(back, _)| from + back + 1)
        .collect()
}

// The window around a change: `WINDOW` lines above it and below it, 0-based and inclusive.
fn window(at: usize, end: usize, total: usize) -> (usize, usize) {
    (
        at.saturating_sub(WINDOW).max(1) - 1,
        (end + WINDOW).min(total) - 1,
    )
}

// The one line a model reads back: the lines that went, the lines that came, what moved below.
pub(crate) fn receipt(from: usize, old_lines: usize, new_lines: usize, total: usize) -> String {
    let to = from + old_lines.saturating_sub(1);
    let below = new_lines as i64 - old_lines as i64;
    format!(
        "replaced lines {from}-{to} ({old_lines}) with {new_lines} lines; below {below:+}; \
         file now {total} lines"
    )
}

// Which line (1-based) a byte position sits on.
pub(crate) fn line_of(text: &str, pos: usize) -> usize {
    text[..pos.min(text.len())].matches('\n').count() + 1
}

// A text's lines, with no empty line for the newline that ends it.
pub(crate) fn lines_of(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<String> = text.split('\n').map(str::to_string).collect();
    if text.ends_with('\n') {
        lines.pop();
    }
    lines
}
