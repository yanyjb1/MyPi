// The CP sorter: which lines are key, what each piece is, and where it ends. One grammar for both halves: C's code is the part of C++ that parses most simply.

use std::path::Path;

use super::LANGUAGE;
use super::scan::{self, Blank};
use crate::server::agent::tools::fs::lang::{Line, Table};

// The suffixes this sorter answers to; a header belongs to no half in particular, which is why there is one language and not two.
pub(crate) const SUFFIXES: &[&str] = &[
    "c", "h", "cc", "hh", "cpp", "hpp", "cxx", "hxx", "inl", "ipp",
];

// The suffixes a compiler compiles on its own; every other suffix this language claims is a header.
const UNITS: &[&str] = &["c", "cc", "cpp", "cxx"];

// Whether a file is a translation unit rather than a header.
pub(super) fn source(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| UNITS.contains(&ext))
}

// Words that open a piece of their own and name what it is.
const HEADS: &[&str] = &[
    "namespace",
    "class",
    "struct",
    "union",
    "enum",
    "using",
    "typedef",
];

// Words that open a block without naming a piece, so a line that leads with one is no declaration.
const CONTROL: &[&str] = &[
    "if",
    "else",
    "for",
    "while",
    "do",
    "switch",
    "catch",
    "try",
    "return",
    "case",
    "default",
    "goto",
    "throw",
    "new",
    "delete",
    "break",
    "continue",
    "sizeof",
    "static_assert",
];

// Words that may stand before a declaration's own head.
const MODIFIERS: &[&str] = &[
    "inline",
    "static",
    "explicit",
    "virtual",
    "constexpr",
    "consteval",
    "constinit",
    "friend",
    "const",
    "volatile",
    "mutable",
    "register",
    "thread_local",
];

// A function at least this long hands out key lines inside its body.
const LONG: usize = 15;

// One file's line table.
pub(crate) fn table(_path: &Path, text: &str) -> Table {
    let raw: Vec<&str> = text.lines().collect();
    let mut code = scan::strip(text, token);
    directives(&raw, &mut code);
    let ends = scan::ends(&code, '{', '}');
    let mut table = Table::new(raw.len(), &LANGUAGE);
    pieces(&raw, &code, &ends, &mut table);
    hats(&raw, &mut table);
    control(&code, &ends, &mut table);
    table
}

// What a piece of this file is: `namespace`, `class`, `enum`, `function`, `using`, `typedef`.
pub(crate) fn shape(line: &Line) -> Option<&str> {
    line.label.as_deref()
}

// Blank every preprocessor line out of the code: a directive's braces are not a body, and a macro body may hold any of them.
fn directives(raw: &[&str], code: &mut [String]) {
    let mut at = 0;
    while at < raw.len() {
        if !raw[at].trim_start().starts_with('#') {
            at += 1;
            continue;
        }
        loop {
            code[at].clear();
            let continues = raw[at].trim_end().ends_with('\\');
            at += 1;
            if !continues || at >= raw.len() {
                break;
            }
        }
    }
}

// Every declaration: the line it opens on, the line it ends on, and what it declares.
fn pieces(raw: &[&str], code: &[String], ends: &[Option<usize>], table: &mut Table) {
    for at in 0..code.len() {
        if code[at].trim_start().is_empty() {
            continue;
        }
        let Some((open, body)) = span(code, at) else {
            continue;
        };
        let header = header_text(raw, at, open);
        let Some(label) = classify(&header) else {
            continue;
        };
        let close = if body {
            ends.get(open).copied().flatten().unwrap_or(open)
        } else {
            open
        };
        table.span(
            at,
            close,
            Some(label.to_string()),
            Some(without_body(&header)),
        );
    }
}

// Comments: a run's first and last line are key, whatever they say.
fn hats(raw: &[&str], table: &mut Table) {
    let mut at = 0;
    while at < raw.len() {
        let line = raw[at].trim_start();
        if !line.starts_with("//") && !line.starts_with("/*") {
            at += 1;
            continue;
        }
        let end = if line.starts_with("/*") && !line.contains("*/") {
            (at + 1..raw.len())
                .find(|probe| raw[*probe].contains("*/"))
                .unwrap_or(at)
        } else {
            let mut end = at;
            while end + 1 < raw.len() && raw[end + 1].trim_start().starts_with("//") {
                end += 1;
            }
            end
        };
        table.span(at, end, None, None);
        at = end + 1;
    }
}

// A long function's own control-flow blocks: the blocks a reader jumps between inside one body.
fn control(code: &[String], ends: &[Option<usize>], table: &mut Table) {
    for (open, close) in scan::frames(code, '{', '}') {
        if close <= open + LONG || !table.line(open).is_some_and(Line::head) {
            continue;
        }
        if table.line(open).and_then(shape) != Some("function") {
            continue;
        }
        for (at, line) in code.iter().enumerate().take(close).skip(open + 1) {
            if table.key(at) || !block_head(line.trim()) {
                continue;
            }
            let end = ends.get(at).copied().flatten().unwrap_or(at);
            table.span(at, end, None, None);
        }
    }
}

// The head word of a header once its modifiers and any leading closing braces are behind it.
fn lead_word(header: &str) -> String {
    let line = header
        .trim_start()
        .trim_start_matches(['}', ';'])
        .trim_start();
    for token in line.split_whitespace() {
        let word = raw_word(token);
        if word.is_empty() {
            continue;
        }
        if MODIFIERS.contains(&word.as_str()) {
            continue;
        }
        return word;
    }
    String::new()
}

// What a declaration is, from its header alone: its own head word when it names one, `function` when it opens a callable, nothing for a statement.
fn classify(header: &str) -> Option<&'static str> {
    let line = header
        .trim_start()
        .trim_start_matches(['}', ';'])
        .trim_start();
    let word = lead_word(line);
    if let Some(head) = HEADS.iter().find(|head| **head == word.as_str()) {
        return Some(head);
    }
    // `extern "C" {` wraps declarations without declaring one.
    if word == "extern" && line.contains("\"C\"") {
        return Some("extern");
    }
    if CONTROL.contains(&word.as_str()) {
        return None;
    }
    function(line)
}

// Whether a header opens a callable: a body after its parameters, or a prototype that ends them.
fn function(line: &str) -> Option<&'static str> {
    let paren = line.find('(')?;
    let before = &line[..paren];
    if line.contains("operator") {
        return Some("function");
    }
    // A call on the way to somewhere else is a statement, not a declaration.
    if [
        "=", "<<", ">>", "+=", "-=", "*=", "/=", "++", "--", "==", "!=",
    ]
    .iter()
    .any(|mark| before.contains(mark))
    {
        return None;
    }
    let opens = line.trim_end().ends_with('{');
    let declares = line.trim_end().ends_with(';') && !before.trim().is_empty();
    (opens || declares).then_some("function")
}

// A header's ends: the line its body opens on, and whether it has a body at all. Parentheses and brackets are counted, so a wrapped parameter list stays inside the header.
fn span(code: &[String], start: usize) -> Option<(usize, bool)> {
    let mut paren = 0i64;
    let mut square = 0i64;
    for (at, line) in code.iter().enumerate().skip(start) {
        for c in line.chars() {
            match c {
                '(' => paren += 1,
                ')' => paren -= 1,
                '[' => square += 1,
                ']' => square -= 1,
                '{' if paren <= 0 && square <= 0 => return Some((at, true)),
                ';' if paren <= 0 && square <= 0 => return Some((at, false)),
                _ => {}
            }
        }
    }
    None
}

// A header's lines joined into one, so a wrapped signature reads as one line.
// A header's lines joined into one, so a wrapped signature reads as one line. The line the body opens on is cut at its brace, so a one-line body never joins the signature.
fn header_text(raw: &[&str], head: usize, open: usize) -> String {
    let mut text = String::new();
    for line in raw.iter().take(open).skip(head) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(line);
    }
    let last = raw.get(open).map_or("", |line| line.trim());
    let last = match last.find('{') {
        Some(at) => last[..=at].trim_end_matches(|c: char| c != '{'),
        None => last,
    };
    if !last.is_empty() {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(last);
    }
    text
}

// A header with its body's own punctuation behind it: `int f() {` → `int f()`, `class Foo : public Bar {` → `class Foo : public Bar`.
fn without_body(header: &str) -> String {
    let mut exact = header.trim();
    for tail in ["{", ";"] {
        if let Some(rest) = exact.strip_suffix(tail) {
            exact = rest.trim_end();
        }
    }
    exact.to_string()
}

// A line that opens a block inside a long function: a keyword, or a brace that ends the line behind an expression.
fn block_head(line: &str) -> bool {
    let line = line.trim_start_matches('}').trim_start();
    for word in ["if", "else", "for", "while", "switch", "do", "try", "catch"] {
        if let Some(rest) = line.strip_prefix(word)
            && (rest.is_empty() || rest.starts_with([' ', '(']))
        {
            return true;
        }
    }
    line.ends_with('{')
}

// A token's leading identifier: `std::vector<int>` → `std`, `~Foo()` → the empty word.
fn raw_word(token: &str) -> String {
    token
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

// The CP delimiters: comments, quoted and char literals, and the raw strings C++ adds.
fn token(blank: &mut Blank, c: char) -> bool {
    match c {
        '/' if blank.peek(1) == Some('/') => {
            blank.skip(2);
            blank.skip_to_eol();
            blank.gap();
        }
        '/' if blank.peek(1) == Some('*') => {
            blank.skip(2);
            blank.skip_block();
            blank.gap();
        }
        '"' => {
            blank.skip(1);
            blank.gap();
            blank.skip_quote('"');
        }
        '\'' => return quote(blank),
        'R' if blank.peek(1) == Some('"') => scan::raw(blank, 1),
        'L' | 'u' | 'U' => return prefixed(blank, c),
        _ => return false,
    }
    true
}

// A char literal closes within a few characters; a digit's own separator never opens one.
fn quote(blank: &mut Blank) -> bool {
    if blank
        .last()
        .is_some_and(|c| c.is_alphanumeric() || c == '_')
    {
        blank.hold('\'');
        return true;
    }
    if blank.peek(1) == Some('\\') {
        for k in 2..14 {
            match blank.peek(k) {
                Some('\'') => {
                    blank.skip(k + 1);
                    blank.gap();
                    return true;
                }
                Some('\n') | None => break,
                _ => {}
            }
        }
    } else if blank.peek(2) == Some('\'') {
        blank.skip(3);
        blank.gap();
        return true;
    }
    blank.hold('\'');
    true
}

// `L"…"`, `u8"…"`, `U"…"`, and the raw strings behind the same prefixes: `LR"(…)"`, `u8R"(…)"`.
fn prefixed(blank: &mut Blank, c: char) -> bool {
    let mut k = 1;
    if c == 'u' && blank.peek(1) == Some('8') {
        k = 2;
    }
    if blank.peek(k) == Some('R') && blank.peek(k + 1) == Some('"') {
        scan::raw(blank, k + 1);
        return true;
    }
    match blank.peek(k) {
        Some('"') => {
            blank.skip(k + 1);
            blank.gap();
            blank.skip_quote('"');
            true
        }
        Some('\'') => {
            blank.skip(k);
            quote(blank)
        }
        _ => {
            blank.hold(c);
            true
        }
    }
}
