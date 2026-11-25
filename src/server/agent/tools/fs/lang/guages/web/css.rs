// CSS: its own scanner, and the sorter that reads its rule blocks and its at-rule blocks.

use super::LANGUAGE;
use crate::server::agent::tools::fs::lang::Table;

// One text read by the scan: every line's code in place, the braces that shape it, and which lines carry code of their own.
pub(crate) struct Scanned {
    pub(crate) code: Vec<String>,

    // One entry per line, left to right: the column where a brace opens, closes, or a declaration ends.
    pub(crate) events: Vec<Vec<(usize, char)>>,

    // One entry per line: whether the line carries code, or only the inside of a comment or a string.
    pub(crate) live: Vec<bool>,
}

// One text being blanked.
struct Blank {
    chars: Vec<char>,
    at: usize,
    line: String,
    events: Vec<(usize, char)>,
    column: usize,

    // Whether this line has written a code character yet.
    live: bool,
    core: Vec<bool>,
    code: Vec<String>,
    all: Vec<Vec<(usize, char)>>,
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
        if !c.is_whitespace() {
            self.live = true;
        }
        if matches!(c, '{' | '}' | ';') {
            self.events.push((self.column, c));
        }
        self.line.push(c);
        self.column += 1;
        self.at += 1;
    }

    // The blank left where a comment or a string stood.
    fn gap(&mut self) {
        if !self.line.ends_with(' ') {
            self.line.push(' ');
            self.column += 1;
        }
    }

    fn eol(&mut self) {
        self.code.push(std::mem::take(&mut self.line));
        self.all.push(std::mem::take(&mut self.events));
        self.core.push(std::mem::replace(&mut self.live, false));
        self.at += 1;
        self.column = 0;
    }

    // Step past a quoted string; a CSS string ends at its own line even when it never closes.
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

    fn finish(mut self) -> Scanned {
        if !self.ended && !self.chars.is_empty() {
            self.code.push(std::mem::take(&mut self.line));
            self.all.push(std::mem::take(&mut self.events));
            self.core.push(self.live);
        }
        Scanned {
            code: self.code,
            events: self.all,
            live: self.core,
        }
    }
}

// Blank one text, keeping its line count.
pub(super) fn strip(text: &str) -> Scanned {
    let mut blank = Blank {
        chars: text.chars().collect(),
        at: 0,
        line: String::new(),
        events: Vec::new(),
        column: 0,
        live: false,
        core: Vec::new(),
        code: Vec::new(),
        all: Vec::new(),
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

// CSS's delimiters: a comment, a string, and the braces a rule opens and closes.
fn token(blank: &mut Blank, c: char) -> bool {
    match c {
        '/' if blank.peek(1) == Some('*') => {
            blank.skip(2);
            while blank.at < blank.chars.len()
                && !(blank.peek(0) == Some('*') && blank.peek(1) == Some('/'))
            {
                if blank.peek(0) == Some('\n') {
                    blank.eol();
                } else {
                    blank.at += 1;
                }
            }
            blank.skip(2);
            blank.gap();
        }
        '"' | '\'' => {
            blank.skip(1);
            blank.gap();
            blank.skip_quote(c);
        }
        '{' | '}' | ';' => blank.hold(c),
        _ => return false,
    }
    true
}

// The brace pairs of one text, as the lines they open and close on.
fn frames(scanned: &Scanned) -> Vec<(usize, usize)> {
    let mut stack: Vec<usize> = Vec::new();
    let mut out: Vec<(usize, usize)> = Vec::new();
    for (at, events) in scanned.events.iter().enumerate() {
        for (_, c) in events {
            match c {
                '{' => stack.push(at),
                '}' => {
                    if let Some(open) = stack.pop() {
                        out.push((open, at));
                    }
                }
                _ => {}
            }
        }
    }
    out.sort_unstable();
    out
}

// One file's line table: every rule block and at-rule block, nested ones included. A block's head is its selector or at-rule; its declarations are ordinary lines.
pub(crate) fn table(text: &str) -> Table {
    let scanned = strip(text);
    let raw: Vec<&str> = text.lines().collect();
    let mut table = Table::new(scanned.code.len(), &LANGUAGE);
    for (open, close) in frames(&scanned) {
        let Some(head) = head_line(&scanned, open) else {
            continue;
        };
        let exact = head_text(&raw, head, open);
        if exact.is_empty() {
            continue;
        }
        let label = exact
            .split([' ', '(', ',', '{'])
            .next()
            .filter(|word| word.starts_with('@'))
            .unwrap_or("rule");
        table.span(head, close, Some(label.to_string()), Some(exact));
    }
    table
}

// The line a block's own head stands on: the line holding its opening brace, or the line just above it when the brace stands alone.
fn head_line(scanned: &Scanned, open: usize) -> Option<usize> {
    if !scanned.code[open]
        .trim()
        .trim_start_matches('{')
        .trim()
        .is_empty()
    {
        return Some(open);
    }
    if open == 0 {
        return None;
    }
    let code = scanned.code[open - 1].trim();
    (!code.is_empty() && !code.starts_with('}') && !code.starts_with('{')).then_some(open - 1)
}

// A block's head as it reads, taken from the file's own text so a quoted attribute keeps its quotes: its selector or at-rule, folded over the lines it runs onto.
fn head_text(raw: &[&str], at: usize, open: usize) -> String {
    let mut parts: Vec<String> = Vec::new();
    for line in at..=open {
        let Some(code) = raw.get(line) else {
            break;
        };
        let text = code.trim();
        let head = text.split('{').next().unwrap_or(text).trim();
        if !head.is_empty() {
            parts.push(head.to_string());
        }
        if text.contains('{') {
            break;
        }
    }
    parts
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

// Every stylesheet this file pulls in, as written, with the line it stands on. A path is a quoted string, which the scan blanks out, so the file's own text is read on the lines the scan found carrying code.
pub(super) fn imports(scanned: &Scanned, text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (at, raw) in text.lines().enumerate() {
        if !scanned.live.get(at).copied().unwrap_or(false) {
            continue;
        }
        let line = raw.trim();
        let Some(rest) = line.strip_prefix("@import") else {
            continue;
        };
        let rest = rest.trim_start();
        let rest = rest
            .strip_prefix("url(")
            .map(str::trim_start)
            .unwrap_or(rest);
        let rest = rest.trim_start_matches(['"', '\'']).trim_start();
        let end = rest
            .find(|c: char| c == '"' || c == '\'' || c == ')' || c == ';' || c.is_whitespace())
            .unwrap_or(rest.len());
        if end > 0 {
            out.push((at, rest[..end].to_string()));
        }
    }
    out
}

// The ids and classes one stylesheet's selectors name, with the line each rule opens on.
pub(super) fn anchors(table: &Table) -> Vec<(char, String, usize)> {
    let mut out: Vec<(char, String, usize)> = Vec::new();
    for (at, line) in table.pieces() {
        let Some(exact) = line.exact.as_deref() else {
            continue;
        };
        for part in exact.split([',', ' ', '>', '+', '~', '(', ')']) {
            let part = part.trim();
            let Some(kind) = part.chars().next().filter(|c| *c == '#' || *c == '.') else {
                continue;
            };
            let name: String = part[1..]
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
                .collect();
            if name.is_empty() || out.contains(&(kind, name.clone(), at)) {
                continue;
            }
            out.push((kind, name, at));
        }
    }
    out
}
