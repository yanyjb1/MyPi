// Native JavaScript: its own scanner, and the sorter that reads its declarations, methods and control blocks.

use super::LANGUAGE;
use crate::server::agent::tools::fs::lang::Table;

// Words after which a `/` opens a regular expression rather than dividing.
const BEFORE: &[&str] = &[
    "await",
    "case",
    "delete",
    "do",
    "else",
    "in",
    "instanceof",
    "new",
    "of",
    "return",
    "throw",
    "typeof",
    "void",
    "yield",
];

// A definition at least this long hands out key lines inside its own body.
const LONG: usize = 15;

// How far a declaration's header may run before the scan stops looking for its block.
const HEAD: usize = 20;

// Words that open a control block inside a long body.
const CONTROL: &[&str] = &[
    "catch", "do", "else", "finally", "for", "if", "switch", "try", "while",
];

// Modifiers that may stand in front of a class member's name.
const MODIFIERS: &[&str] = &[
    "static",
    "async",
    "get",
    "set",
    "public",
    "private",
    "protected",
    "readonly",
    "override",
];

// One text read by the scan: every line's code, and the brackets and semicolons that shape it, left to right.
pub(crate) struct Scanned {
    // Line count unchanged; a comment and a literal stand as a blank.
    pub(crate) code: Vec<String>,

    // One entry per line: the column where a bracket opens, closes, or a statement ends.
    pub(crate) events: Vec<Vec<(usize, char)>>,

    // One entry per line: whether the line carries code of its own, or only the inside of a comment or a literal.
    pub(crate) live: Vec<bool>,
}

// One text being blanked.
struct Blank {
    chars: Vec<char>,
    at: usize,
    line: String,
    events: Vec<(usize, char)>,
    column: usize,

    // The last code character written on this line, and the identifier ending there.
    last: Option<char>,
    word: String,

    // How deep inside `${ … }` the scan stands, one counter per open template expression.
    holes: Vec<i64>,

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

    // Write one code character, in place.
    fn hold(&mut self, c: char) {
        if !c.is_whitespace() {
            self.last = Some(c);
            self.live = true;
        }
        if c.is_alphanumeric() || c == '_' || c == '$' {
            self.word.push(c);
        } else if !c.is_whitespace() {
            // Blank space stands between a keyword and the value after it, so only punctuation ends the word, and a `/` past `return` still sees it.
            self.word.clear();
        }
        if matches!(c, '{' | '}' | ';') {
            self.events.push((self.column, c));
        }
        self.line.push(c);
        self.column += 1;
        self.at += 1;
    }

    // The blank left where a comment or a literal stood: the character before it still says what may follow.
    fn gap(&mut self) {
        if !self.line.ends_with(' ') {
            self.line.push(' ');
            self.column += 1;
        }
        self.word.clear();
    }

    // The blank left where a literal stood: an operand just ended, so a `/` here divides.
    fn literal(&mut self) {
        self.gap();
        self.last = Some('_');
    }

    // End the current line and step over its newline.
    fn eol(&mut self) {
        self.code.push(std::mem::take(&mut self.line));
        self.all.push(std::mem::take(&mut self.events));
        self.core.push(std::mem::replace(&mut self.live, false));
        self.at += 1;
        self.column = 0;
        self.last = None;
        self.word.clear();
    }

    fn skip_to_eol(&mut self) {
        while self.at < self.chars.len() && self.chars[self.at] != '\n' {
            self.at += 1;
        }
    }

    // Step past a quoted string: an escaped character holds nothing, an unclosed quote ends at the line.
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

    // Step past a regular expression body, up to its closing slash and the flags behind it.
    fn skip_regex(&mut self) {
        let mut class = false;
        while self.at < self.chars.len() {
            match self.peek(0) {
                Some('\n') => return,
                Some('\\') => self.skip(2),
                Some('[') => {
                    class = true;
                    self.at += 1;
                }
                Some(']') => {
                    class = false;
                    self.at += 1;
                }
                Some('/') if !class => {
                    self.at += 1;
                    while self.peek(0).is_some_and(|c| c.is_ascii_alphabetic()) {
                        self.at += 1;
                    }
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
        last: None,
        word: String::new(),
        holes: Vec::new(),
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

// JavaScript's delimiters: a comment, a quoted string, a template, a regular expression, and the brackets a template hole holds.
fn token(blank: &mut Blank, c: char) -> bool {
    match c {
        '/' if blank.peek(1) == Some('/') => {
            blank.skip_to_eol();
            blank.gap();
        }
        '/' if blank.peek(1) == Some('*') => {
            blank.skip(2);
            while blank.at < blank.chars.len()
                && !(blank.peek(0) == Some('*') && blank.peek(1) == Some('/'))
            {
                if blank.peek(0) == Some('\n') {
                    blank.gap();
                    blank.eol();
                } else {
                    blank.at += 1;
                }
            }
            blank.skip(2);
            blank.gap();
        }
        '/' if regex_here(blank) => {
            blank.skip(1);
            blank.literal();
            blank.skip_regex();
        }
        '"' | '\'' => {
            blank.skip(1);
            blank.literal();
            blank.skip_quote(c);
        }
        '`' => {
            blank.skip(1);
            blank.literal();
            template(blank);
        }
        '{' => {
            if let Some(depth) = blank.holes.last_mut() {
                *depth += 1;
            }
            blank.hold(c);
        }
        '}' => {
            if let Some(depth) = blank.holes.last_mut()
                && *depth > 0
            {
                *depth -= 1;
                blank.hold(c);
                return true;
            }
            if blank.holes.pop().is_some() {
                blank.skip(1);
                blank.literal();
                template(blank);
                return true;
            }
            blank.hold(c);
        }
        ';' => blank.hold(c),
        _ => return false,
    }
    true
}

// Whether the `/` standing here opens a regular expression: only where an operand may begin.
fn regex_here(blank: &Blank) -> bool {
    match blank.last {
        None => true,
        Some(c) if c.is_alphanumeric() || c == '_' || c == '$' => {
            BEFORE.contains(&blank.word.as_str())
        }
        Some(')' | ']') => false,
        Some(_) => true,
    }
}

// Step past a template's own text, up to its closing backtick or the next `${`, which hands the scan back to code.
fn template(blank: &mut Blank) {
    while blank.at < blank.chars.len() {
        match blank.peek(0) {
            Some('\\') => blank.skip(2),
            Some('`') => {
                blank.at += 1;
                return;
            }
            Some('$') if blank.peek(1) == Some('{') => {
                blank.skip(2);
                blank.holes.push(0);
                return;
            }
            Some('\n') => {
                blank.gap();
                blank.eol();
            }
            Some(_) => blank.at += 1,
            None => return,
        }
    }
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

// The line the block opening on `at` ends on, for the outermost brace that line opens.
fn closed(frames: &[(usize, usize)], at: usize) -> Option<usize> {
    frames
        .iter()
        .filter(|(open, _)| *open == at)
        .map(|(_, close)| *close)
        .max()
}

// The line the block a declaration opens ends on; `None` when the declaration carries no block of its own.
fn body(scanned: &Scanned, frames: &[(usize, usize)], at: usize) -> Option<usize> {
    for (line, events) in scanned
        .events
        .iter()
        .enumerate()
        .take(scanned.code.len().min(at + HEAD))
        .skip(at)
    {
        for (_, c) in events {
            match c {
                '{' => return closed(frames, line),
                ';' => return None,
                _ => {}
            }
        }
    }
    None
}

// How far one line is pushed in; `None` for a blank one.
fn indent(line: &str) -> Option<usize> {
    (!line.trim().is_empty()).then(|| line.chars().take_while(|c| c.is_whitespace()).count())
}

// The line's first word, once its leading blanks are behind it.
fn first_word(code: &str) -> &str {
    let rest = code.trim_start();
    let end = rest
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
        .unwrap_or(rest.len());
    &rest[..end]
}

// One file's line table.
pub(crate) fn table(text: &str) -> Table {
    let scanned = strip(text);
    let frames = frames(&scanned);
    let mut table = Table::new(scanned.code.len(), &LANGUAGE);
    pieces(&scanned, &frames, &mut table);
    members(&scanned, &frames, &mut table);
    control(&scanned, &frames, &mut table);
    table
}

// Every top-level declaration: its block, its label and the name it binds.
fn pieces(scanned: &Scanned, frames: &[(usize, usize)], table: &mut Table) {
    for at in 0..scanned.code.len() {
        if indent(&scanned.code[at]) != Some(0) {
            continue;
        }
        let code = scanned.code[at].trim();
        let Some(kind) = head_of(code) else {
            continue;
        };
        let end = body(scanned, frames, at).unwrap_or(at);
        table.span(at, end, Some(kind.to_string()), Some(exact(kind, code)));
    }
}

// The kind of declaration this line opens: `fn`, `class`, `var`, `import`.
fn head_of(code: &str) -> Option<&'static str> {
    let mut rest = code;
    loop {
        let word = first_word(rest);
        if !matches!(word, "export" | "default" | "async" | "declare") {
            break;
        }
        rest = rest[word.len()..].trim_start();
    }
    match first_word(rest) {
        "function" | "class" => Some(if first_word(rest) == "function" {
            "fn"
        } else {
            "class"
        }),
        "const" | "let" | "var" => Some("var"),
        "import" => Some("import"),
        _ => None,
    }
}

// What a declaration's line reads as its own name: the header up to the body, the parameters or the assigned value.
fn exact(kind: &str, code: &str) -> String {
    let cut = if kind == "import" {
        code
    } else {
        code.split('{').next().unwrap_or(code)
    };
    let cut = cut.trim().trim_end_matches(';').trim();
    let cut = match kind {
        "fn" | "class" => cut.split('(').next().unwrap_or(cut),
        "var" => cut.split('=').next().unwrap_or(cut),
        _ => cut,
    };
    cut.split_whitespace().collect::<Vec<_>>().join(" ")
}

// The first level of one block's body, as far in as its own statements stand.
fn inner(scanned: &Scanned, at: usize, end: usize) -> Option<usize> {
    (at + 1..=end)
        .filter(|line| {
            let code = scanned.code[*line].trim();
            !code.is_empty() && !code.starts_with('}')
        })
        .filter_map(|line| indent(&scanned.code[line]))
        .min()
}

// The methods of every class: the statements of its own body that declare one.
fn members(scanned: &Scanned, frames: &[(usize, usize)], table: &mut Table) {
    let classes: Vec<(usize, usize)> = table
        .pieces()
        .into_iter()
        .filter(|(_, line)| line.label.as_deref() == Some("class"))
        .filter_map(|(at, _)| table.end(at).map(|end| (at, end)))
        .collect();
    for (at, end) in classes {
        let Some(depth) = inner(scanned, at, end) else {
            continue;
        };
        for line in at + 1..=end {
            if table.key(line) || indent(&scanned.code[line]) != Some(depth) {
                continue;
            }
            let code = scanned.code[line].trim();
            if !method(code) {
                continue;
            }
            let stop = body(scanned, frames, line).unwrap_or(line);
            table.span(
                line,
                stop,
                Some("method".to_string()),
                Some(exact("fn", code)),
            );
        }
    }
}

// Whether this statement declares a class member: a modifier run, then a name, then its parameters.
fn method(code: &str) -> bool {
    let mut rest = code.trim_start_matches('*').trim_start();
    loop {
        let word = first_word(rest);
        if !MODIFIERS.contains(&word) {
            break;
        }
        rest = rest[word.len()..].trim_start();
    }
    let name = rest.trim_start_matches('#');
    let end = name
        .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$'))
        .unwrap_or(name.len());
    let after = name[end..].trim_start();
    end > 0 && after.starts_with('(')
}

// The control blocks inside a long definition's body.
fn control(scanned: &Scanned, frames: &[(usize, usize)], table: &mut Table) {
    let long: Vec<(usize, usize)> = table
        .pieces()
        .into_iter()
        .filter(|(_, line)| matches!(line.label.as_deref(), Some("fn" | "method")))
        .filter_map(|(at, _)| table.end(at).map(|end| (at, end)))
        .collect();
    for (at, end) in long {
        if end <= at + LONG {
            continue;
        }
        let Some(depth) = inner(scanned, at, end) else {
            continue;
        };
        for line in at + 1..=end {
            if table.key(line) || indent(&scanned.code[line]) != Some(depth) {
                continue;
            }
            let rest = scanned.code[line]
                .trim()
                .trim_start_matches('}')
                .trim_start();
            if !CONTROL.contains(&first_word(rest)) {
                continue;
            }
            let stop = body(scanned, frames, line).unwrap_or(line);
            table.span(line, stop, None, None);
        }
    }
}

// Every line of one text that carries code, with its own number: a line standing inside a comment or a literal names no module and calls nobody. The text itself, not the scan, since a module path is a string the scan blanks out.
fn code_lines<'a>(scanned: &Scanned, text: &'a str) -> impl Iterator<Item = (usize, &'a str)> {
    text.lines()
        .enumerate()
        .filter(|(at, _)| scanned.live.get(*at).copied().unwrap_or(false))
}

// Every module this file imports or re-exports, as written, with the line it stands on.
pub(super) fn refs(scanned: &Scanned, text: &str) -> Vec<(usize, String, Vec<String>)> {
    let mut out = Vec::new();
    for (at, raw) in code_lines(scanned, text) {
        let line = raw.trim();
        if !(line.starts_with("import") || line.starts_with("export")) {
            continue;
        }
        let Some(rest) = after_from(line) else {
            continue;
        };
        let Some(wrote) = quoted(rest) else {
            continue;
        };
        out.push((at, wrote, named(line)));
    }
    out
}

// What stands after `from`, when this line carries one.
fn after_from(line: &str) -> Option<&str> {
    let at = line.find(" from ")?;
    Some(line[at + " from ".len()..].trim_start())
}

// The first quoted text of a fragment.
fn quoted(text: &str) -> Option<String> {
    let quote = text.find(['"', '\''])?;
    let mark = text[quote..].chars().next()?;
    let tail = &text[quote + mark.len_utf8()..];
    let end = tail.find(mark)?;
    Some(tail[..end].to_string())
}

// The names a module line brings in or hands on: `{ a, b as c }` reads `a`, `c`.
fn named(line: &str) -> Vec<String> {
    let Some(open) = line.find('{') else {
        return Vec::new();
    };
    let Some(close) = line[open..].find('}') else {
        return Vec::new();
    };
    line[open + 1..open + close]
        .split(',')
        .filter_map(|item| {
            let item = item.trim();
            if item.is_empty() || item == "*" {
                return None;
            }
            let name = item
                .rsplit_once(" as ")
                .map_or(item, |(_, alias)| alias.trim());
            (!name.is_empty()).then(|| name.to_string())
        })
        .collect()
}

// Every name this file exports.
pub(super) fn exports(scanned: &Scanned, text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (_, raw) in code_lines(scanned, text) {
        let line = raw.trim();
        let Some(rest) = line.strip_prefix("export ") else {
            continue;
        };
        let rest = rest.trim_start();
        if rest.starts_with('{') {
            for name in named(rest) {
                if !out.contains(&name) {
                    out.push(name);
                }
            }
            continue;
        }
        let rest = rest.strip_prefix("default ").unwrap_or(rest).trim_start();
        let rest = rest.strip_prefix("async ").unwrap_or(rest).trim_start();
        for word in ["function", "class", "const", "let", "var"] {
            if let Some(tail) = rest.strip_prefix(word) {
                let name: String = tail
                    .trim_start()
                    .trim_start_matches('*')
                    .trim_start()
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                    .collect();
                if !name.is_empty() && !out.contains(&name) {
                    out.push(name);
                }
                break;
            }
        }
    }
    out
}
