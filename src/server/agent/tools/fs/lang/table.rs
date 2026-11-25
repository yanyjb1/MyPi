// One file's line table: each line's stamp, the line it pairs with, and what it names.

use super::Language;

// What one line does to the structure.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Stamp {
    // Nothing structural: a blank line, a statement, the inside of a folded run.
    Plain,
    // A piece opens here.
    Open,
    // The piece opened above ends on this line.
    Close,
    // One whole piece on one line: `use a::b;`, `struct S;`.
    Single,
}

// One line: its stamp, the shape it reads by, and the name it binds.
pub(crate) struct Line {
    pub(crate) stamp: Stamp,
    // The syntax shape, never a name: `fn`, `impl`, `impl a for b`, `use`.
    pub(crate) label: Option<String>,
    // The one thing this piece is: `new`, `Env`, `Display for Foo`.
    pub(crate) exact: Option<String>,
    // The other end of the piece: an opening line's closing line, and back.
    pub(crate) mate: Option<usize>,
}

impl Line {
    fn bare() -> Self {
        Self {
            stamp: Stamp::Plain,
            label: None,
            exact: None,
            mate: None,
        }
    }

    // Whether a piece starts on this line.
    pub(crate) fn head(&self) -> bool {
        matches!(self.stamp, Stamp::Open | Stamp::Single)
    }
}

// One file's lines, and who holds whom.
pub(crate) struct Table {
    lines: Vec<Line>,
    // The language that wrote it; `None` for a text no sorter takes.
    language: Option<&'static Language>,
}

impl Table {
    // `n` lines of one language, none of them a piece yet.
    pub(crate) fn new(n: usize, language: &'static Language) -> Self {
        Self {
            lines: (0..n).map(|_| Line::bare()).collect(),
            language: Some(language),
        }
    }

    // A text no sorter takes: every line stands alone, so every line is a key line.
    pub(crate) fn plain(text: &str) -> Self {
        let mut table = Self {
            lines: (0..text.lines().count()).map(|_| Line::bare()).collect(),
            language: None,
        };
        for line in &mut table.lines {
            line.stamp = Stamp::Single;
        }
        table
    }

    // The language that wrote this table.
    pub(crate) fn language(&self) -> Option<&'static Language> {
        self.language
    }

    pub(crate) fn stamp(&self, at: usize) -> Stamp {
        self.lines.get(at).map_or(Stamp::Plain, |line| line.stamp)
    }

    // Whether this line carries a number: any stamp but plain.
    pub(crate) fn key(&self, at: usize) -> bool {
        self.stamp(at) != Stamp::Plain
    }

    pub(crate) fn line(&self, at: usize) -> Option<&Line> {
        self.lines.get(at)
    }

    // The line a piece ends on; `None` where no piece starts.
    pub(crate) fn end(&self, at: usize) -> Option<usize> {
        let line = self.lines.get(at)?;
        line.head().then(|| line.mate.unwrap_or(at))
    }

    // One piece: the line it opens on, and the line it ends on.
    pub(crate) fn span(
        &mut self,
        at: usize,
        end: usize,
        label: Option<String>,
        exact: Option<String>,
    ) {
        if end <= at {
            self.set(at, Stamp::Single, label, exact, Some(at));
            return;
        }
        self.set(at, Stamp::Open, label, exact, Some(end));
        // A line that already opens a piece of its own keeps it: `} else if cond {`.
        if let Some(line) = self.lines.get_mut(end)
            && line.stamp == Stamp::Plain
        {
            line.stamp = Stamp::Close;
            line.mate = Some(at);
        }
    }

    // One piece that reaches to `end` without keying that line: a heading's section ends on ordinary prose, and prose carries no number of its own.
    pub(crate) fn reach(
        &mut self,
        at: usize,
        end: usize,
        label: Option<String>,
        exact: Option<String>,
    ) {
        if end <= at {
            self.set(at, Stamp::Single, label, exact, Some(at));
            return;
        }
        self.set(at, Stamp::Open, label, exact, Some(end));
    }

    fn set(
        &mut self,
        at: usize,
        stamp: Stamp,
        label: Option<String>,
        exact: Option<String>,
        mate: Option<usize>,
    ) {
        if let Some(line) = self.lines.get_mut(at) {
            line.stamp = stamp;
            line.label = label;
            line.exact = exact;
            line.mate = mate;
        }
    }

    // The innermost piece holding this line, with the line it opens on.
    pub(crate) fn innermost(&self, at: usize) -> Option<(usize, &Line)> {
        self.lines
            .iter()
            .enumerate()
            .filter(|(open, line)| holds(line, *open, at))
            .max_by_key(|(open, _)| *open)
    }

    // Every piece holding this line, outermost first, with the line each opens on.
    pub(crate) fn around(&self, at: usize) -> Vec<(usize, &Line)> {
        self.lines
            .iter()
            .enumerate()
            .filter(|(open, line)| holds(line, *open, at))
            .collect()
    }

    // The pieces at the shallowest depth of one line range, in line order.
    pub(crate) fn items(&self, from: usize, to: usize) -> Vec<usize> {
        let to = to.min(self.lines.len().saturating_sub(1));
        if from > to {
            return Vec::new();
        }
        let mut keyed: Vec<(usize, usize)> = Vec::new();
        let mut stems: Vec<usize> = Vec::new();
        for at in 0..=to {
            stems.retain(|end| *end >= at);
            let depth = stems.len();
            let line = self.line(at);
            if at >= from && line.is_some_and(Line::head) {
                keyed.push((at, depth));
            }
            if let Some(line) = line
                && let (Stamp::Open, Some(end)) = (line.stamp, line.mate)
            {
                stems.push(end);
            }
        }
        let Some(least) = keyed.iter().map(|(_, depth)| *depth).min() else {
            return Vec::new();
        };
        keyed
            .into_iter()
            .filter(|(_, depth)| *depth == least)
            .map(|(at, _)| at)
            .collect()
    }

    // Every piece that names itself, in line order.
    pub(crate) fn pieces(&self) -> Vec<(usize, &Line)> {
        self.lines
            .iter()
            .enumerate()
            .filter(|(_, line)| line.head() && line.label.is_some())
            .collect()
    }

    // The pieces this one directly holds, in line order.
    pub(crate) fn inner(&self, at: usize) -> Vec<(usize, &Line)> {
        let Some(end) = self.end(at) else {
            return Vec::new();
        };
        self.items(at + 1, end)
            .into_iter()
            .filter_map(|inner| {
                self.line(inner)
                    .filter(|line| line.label.is_some())
                    .map(|line| (inner, line))
            })
            .collect()
    }
}

// Whether one piece's own lines cover `at`.
fn holds(line: &Line, open: usize, at: usize) -> bool {
    match (line.stamp, line.mate) {
        (Stamp::Open, Some(end)) => open <= at && at <= end,
        (Stamp::Single, _) => open == at,
        _ => false,
    }
}
