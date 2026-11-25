// The Python scan: comments and strings blanked out, brackets tracked so a folded line is known.

// One text read by the scan: every line's code, and which lines merely continue one opened above.
pub(crate) struct Scanned {
    // Line count unchanged; a comment and a string stand as a blank.
    pub(crate) code: Vec<String>,
    // True for a line that opened inside a bracket or a triple-quoted string of a line above it.
    pub(crate) carried: Vec<bool>,
}

// One text being blanked.
struct Blank {
    chars: Vec<char>,
    at: usize,
    line: String,
    // This line began inside a bracket or a triple-quoted string opened above it.
    open: bool,
    depth: i64,
    // Inside a triple-quoted string this very moment.
    text: bool,
    code: Vec<String>,
    carried: Vec<bool>,
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

    // The blank left where a comment or a string stood.
    fn gap(&mut self) {
        if !self.line.ends_with(' ') {
            self.line.push(' ');
        }
    }

    // End the current line and step over its newline; the next line knows what still holds it open.
    fn eol(&mut self) {
        self.code.push(std::mem::take(&mut self.line));
        self.carried.push(self.open);
        self.at += 1;
        self.open = self.depth > 0 || self.text;
    }

    fn skip_to_eol(&mut self) {
        while self.at < self.chars.len() && self.chars[self.at] != '\n' {
            self.at += 1;
        }
    }

    // Step past a one-line string: an escaped character holds nothing, an unclosed quote ends at the line.
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

    // Step past a triple-quoted body; it may run over any number of lines.
    fn skip_triple(&mut self, quote: char) {
        self.text = true;
        while self.at < self.chars.len() {
            match self.peek(0) {
                Some('\n') => self.eol(),
                Some(c)
                    if c == quote && self.peek(1) == Some(quote) && self.peek(2) == Some(quote) =>
                {
                    self.at += 3;
                    self.text = false;
                    self.gap();
                    return;
                }
                _ => self.at += 1,
            }
        }
        self.text = false;
    }

    fn finish(mut self) -> Scanned {
        if !self.ended && !self.chars.is_empty() {
            self.code.push(std::mem::take(&mut self.line));
            self.carried.push(self.open);
        }
        Scanned {
            code: self.code,
            carried: self.carried,
        }
    }
}

// Blank one text, keeping its line count.
pub(crate) fn strip(text: &str) -> Scanned {
    let mut blank = Blank {
        chars: text.chars().collect(),
        at: 0,
        line: String::new(),
        open: false,
        depth: 0,
        text: false,
        code: Vec::new(),
        carried: Vec::new(),
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

// Python's delimiters: a `#` comment, a quoted string, a triple-quoted string, and the brackets that fold a line.
fn token(blank: &mut Blank, c: char) -> bool {
    match c {
        '#' => {
            blank.skip_to_eol();
            blank.gap();
        }
        '"' | '\'' => {
            if blank.peek(1) == Some(c) && blank.peek(2) == Some(c) {
                blank.skip(3);
                blank.gap();
                blank.skip_triple(c);
            } else {
                blank.skip(1);
                blank.gap();
                blank.skip_quote(c);
            }
        }
        '(' | '[' | '{' => {
            blank.depth += 1;
            blank.hold(c);
        }
        ')' | ']' | '}' => {
            blank.depth = (blank.depth - 1).max(0);
            blank.hold(c);
        }
        _ => return false,
    }
    true
}
