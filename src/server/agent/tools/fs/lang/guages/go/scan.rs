// The Go scan: a cursor and a line buffer, driven by Go's own delimiters.

// One text being blanked: comments and literals cut out, code kept, line count unchanged.
pub(super) struct Blank {
    chars: Vec<char>,
    at: usize,
    line: String,
    out: Vec<String>,
    ended: bool,
}

impl Blank {
    // The character `k` ahead; a token that looks past the current one reads it here.
    pub(super) fn peek(&self, k: usize) -> Option<char> {
        self.chars.get(self.at + k).copied()
    }

    pub(super) fn skip(&mut self, n: usize) {
        self.at = (self.at + n).min(self.chars.len());
    }

    // Keep one character as code.
    pub(super) fn hold(&mut self, c: char) {
        self.line.push(c);
        self.at += 1;
    }

    // The blank left where a comment or a literal stood.
    pub(super) fn gap(&mut self) {
        if !self.line.ends_with(' ') {
            self.line.push(' ');
        }
    }

    pub(super) fn eol(&mut self) {
        self.out.push(std::mem::take(&mut self.line));
        self.at += 1;
    }

    pub(super) fn skip_to_eol(&mut self) {
        while self.at < self.chars.len() && self.chars[self.at] != '\n' {
            self.at += 1;
        }
    }

    // Step past a block comment's body; Go's comments do not nest.
    pub(super) fn skip_block(&mut self) {
        while self.at < self.chars.len() {
            match (self.peek(0), self.peek(1)) {
                (Some('\n'), _) => self.eol(),
                (Some('*'), Some('/')) => {
                    self.at += 2;
                    return;
                }
                _ => self.at += 1,
            }
        }
    }

    // Step past an interpreted string or a rune: a backslash holds the next character, and a literal left open at its line ends there.
    pub(super) fn skip_quote(&mut self, close: char) {
        while self.at < self.chars.len() {
            match self.peek(0) {
                Some('\n') => return,
                Some('\\') => self.at += 2,
                Some(c) if c == close => {
                    self.at += 1;
                    return;
                }
                _ => self.at += 1,
            }
        }
    }

    // Step past a raw string: no escape reaches inside it, and it runs over any number of lines.
    pub(super) fn skip_raw(&mut self) {
        while self.at < self.chars.len() {
            match self.peek(0) {
                Some('\n') => self.eol(),
                Some('`') => {
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

// Blank one text; `token` consumes whatever Go calls a comment or a literal.
pub(super) fn strip(text: &str, token: fn(&mut Blank, char) -> bool) -> Vec<String> {
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
        if token(&mut blank, c) {
            continue;
        }
        blank.hold(c);
    }
    blank.finish()
}

// Every `open`-`close` pair, in the order the openers come.
pub(super) fn frames(code: &[String], open: char, close: char) -> Vec<(usize, usize)> {
    let mut stack: Vec<usize> = Vec::new();
    let mut out = Vec::new();
    for (at, line) in code.iter().enumerate() {
        for ch in line.chars() {
            if ch == open {
                stack.push(at);
            } else if ch == close
                && let Some(opened) = stack.pop()
            {
                out.push((opened, at));
            }
        }
    }
    out
}

// Each opening line's closing line; a line that opens twice keeps the wider pair.
pub(super) fn ends(code: &[String], open: char, close: char) -> Vec<Option<usize>> {
    let mut out: Vec<Option<usize>> = vec![None; code.len()];
    for (opened, closed) in frames(code, open, close) {
        if let Some(slot) = out.get_mut(opened) {
            *slot = Some(slot.map_or(closed, |held| held.max(closed)));
        }
    }
    out
}
