// The CP scan: a cursor and a line buffer, driven by the delimiters CP calls comments and literals.

// One text being blanked: comments and literals cut out, code kept, line count unchanged.
pub(super) struct Blank {
    chars: Vec<char>,
    at: usize,
    line: String,
    out: Vec<String>,
    ended: bool,
}

impl Blank {
    // The last character kept as code on this line: a quote right after one is a digit's separator, not a literal.
    pub(super) fn last(&self) -> Option<char> {
        self.line.chars().last()
    }

    // The character `k` ahead; a language that looks past the current one reads it here.
    pub(super) fn peek(&self, k: usize) -> Option<char> {
        self.chars.get(self.at + k).copied()
    }

    // Step over `n` characters without keeping them.
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

    // End the current line and step over its newline.
    pub(super) fn eol(&mut self) {
        self.out.push(std::mem::take(&mut self.line));
        self.at += 1;
    }

    // Step to the end of this line, stopping on its newline.
    pub(super) fn skip_to_eol(&mut self) {
        while self.at < self.chars.len() && self.chars[self.at] != '\n' {
            self.at += 1;
        }
    }

    // Step past a block comment's body; CP has no nested block comments.
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

    // Step past a quoted body: an escape holds the next character, an unescaped `close` ends it.
    pub(super) fn skip_quote(&mut self, close: char) {
        while self.at < self.chars.len() {
            match self.peek(0) {
                Some('\n') => self.eol(),
                Some('\\') => {
                    self.at += 1;
                    if self.peek(0) == Some('\n') {
                        self.eol();
                    } else {
                        self.at += 1;
                    }
                }
                Some(c) if c == close => {
                    self.at += 1;
                    return;
                }
                _ => self.at += 1,
            }
        }
    }

    // Step past a raw string's body, which ends only where `close` says.
    fn skip_run(&mut self, close: &[char]) {
        let width = close.len();
        while self.peek(0).is_some() {
            if self.peek(0) == Some('\n') {
                self.eol();
                continue;
            }
            if (0..width).all(|k| self.peek(k) == Some(close[k])) {
                self.skip(width);
                return;
            }
            self.skip(1);
        }
    }

    fn finish(mut self) -> Vec<String> {
        if !self.ended && !self.chars.is_empty() {
            self.out.push(std::mem::take(&mut self.line));
        }
        self.out
    }
}

// Blank one text; `token` consumes whatever CP calls a comment or a literal.
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

// A raw string's body: `R"mark(…)mark"`. The opening quote stands `quote_at` characters ahead of the cursor.
pub(super) fn raw(blank: &mut Blank, quote_at: usize) {
    blank.skip(quote_at + 1);
    let mut mark = String::new();
    let mut opened = false;
    while let Some(c) = blank.peek(0) {
        if c == '\n' {
            break;
        }
        if c == '(' {
            blank.skip(1);
            opened = true;
            break;
        }
        if c == '"' || c == ')' || mark.chars().count() >= 16 {
            break;
        }
        mark.push(c);
        blank.skip(1);
    }
    blank.gap();
    if !opened {
        return;
    }
    let mut close: Vec<char> = vec![')'];
    close.extend(mark.chars());
    close.push('"');
    blank.skip_run(&close);
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
