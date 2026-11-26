// The R scan: comments and strings blanked, braces paired, line count unchanged.

// One text being blanked.
pub(super) struct Blank {
    chars: Vec<char>,
    at: usize,
    line: String,
    out: Vec<String>,
    ended: bool,
}

impl Blank {
    fn peek(&self, k: usize) -> Option<char> {
        self.chars.get(self.at + k).copied()
    }

    fn skip(&mut self, n: usize) {
        self.at = (self.at + n).min(self.chars.len());
    }

    fn hold(&mut self, c: char) {
        self.line.push(c);
        self.at += 1;
    }

    // The blank left where a comment or a literal stood.
    fn gap(&mut self) {
        if !self.line.ends_with(' ') {
            self.line.push(' ');
        }
    }

    fn eol(&mut self) {
        self.out.push(std::mem::take(&mut self.line));
        self.at += 1;
    }

    fn skip_to_eol(&mut self) {
        while self.at < self.chars.len() && self.chars[self.at] != '\n' {
            self.at += 1;
        }
    }

    // Step past a quoted literal: a backslash holds the next character, and a literal left open ends at its line.
    fn skip_quote(&mut self, close: char) {
        while self.at < self.chars.len() {
            match self.peek(0) {
                Some('\n') => return,
                Some('\\') => self.skip(2),
                Some(c) if c == close => {
                    self.at += 1;
                    return;
                }
                _ => self.at += 1,
            }
        }
    }

    fn finish(mut self) -> Vec<String> {
        if !self.ended && !self.chars.is_empty() {
            self.out.push(std::mem::take(&mut self.line));
        }
        self.out
    }
}

// A byte order mark leads a file an editor on Windows wrote; it belongs to no line of code.
pub(super) fn body(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

// Blank one text, keeping its line count. A backtick name stays code: it names a binding, not a value.
pub(super) fn strip(text: &str) -> Vec<String> {
    let text = body(text);
    let mut blank = Blank {
        chars: text.chars().collect(),
        at: 0,
        line: String::new(),
        out: Vec::new(),
        ended: text.ends_with('\n'),
    };
    while let Some(c) = blank.peek(0) {
        if c == '\n' {
            blank.eol();
            continue;
        }
        match c {
            '#' => {
                blank.skip_to_eol();
                blank.gap();
            }
            '"' | '\'' => {
                blank.skip(1);
                blank.gap();
                blank.skip_quote(c);
            }
            _ => blank.hold(c),
        }
    }
    blank.finish()
}

// Each opening line's closing line; a line that opens twice keeps the wider pair.
pub(super) fn ends(code: &[String], open: char, close: char) -> Vec<Option<usize>> {
    let mut stack: Vec<usize> = Vec::new();
    let mut out: Vec<Option<usize>> = vec![None; code.len()];
    for (at, line) in code.iter().enumerate() {
        for ch in line.chars() {
            if ch == open {
                stack.push(at);
            } else if ch == close
                && let Some(opened) = stack.pop()
                && let Some(slot) = out.get_mut(opened)
            {
                *slot = Some(slot.map_or(at, |held| held.max(at)));
            }
        }
    }
    out
}
