// The Rust sorter: which lines are key, what each piece is, and where it ends.

use std::path::Path;

use super::LANGUAGE;
use super::scan::{self, Blank};
use crate::server::agent::tools::fs::lang::{Line, Table};

// The suffix this sorter answers to.
pub(crate) const SUFFIXES: &[&str] = &["rs"];

// Words that begin a piece once its modifiers are behind it.
const HEADS: &[&str] = &[
    "fn",
    "struct",
    "enum",
    "union",
    "trait",
    "impl",
    "mod",
    "use",
    "const",
    "static",
    "type",
    "extern",
    "macro_rules!",
    "macro",
];

// Words that may stand before a head, or between two of them (`pub async unsafe fn`).
const MODIFIERS: &[&str] = &[
    "pub", "const", "unsafe", "async", "default", "auto", "gen", "extern",
];

// A function at least this long hands out key lines inside its body.
const LONG: usize = 15;

// One file's line table.
pub(crate) fn table(_path: &Path, text: &str) -> Table {
    let raw: Vec<&str> = text.lines().collect();
    let code = scan::strip(text, token);
    let ends = scan::ends(&code, '{', '}');
    let mut table = Table::new(raw.len(), &LANGUAGE);
    pieces(&raw, &code, &ends, &mut table);
    hats(&raw, &code, &mut table);
    control(&code, &ends, &mut table);
    table
}

// Every declaration: the line it opens on, the line it ends on, and what it binds.
fn pieces(raw: &[&str], code: &[String], ends: &[Option<usize>], table: &mut Table) {
    for (at, line) in code.iter().enumerate() {
        let Some(kind) = head_of(line.trim_start()) else {
            continue;
        };
        let Some((open, body)) = span(code, at) else {
            continue;
        };
        let header = header_text(raw, at, open);
        let (label, exact) = names_of(kind, &header);
        let close = if body {
            ends.get(open).copied().flatten().unwrap_or(open)
        } else {
            open
        };
        table.span(at, close, Some(label), exact);
    }
}

// Comment and attribute blocks: their first and last lines are key, whatever they hold.
fn hats(raw: &[&str], code: &[String], table: &mut Table) {
    let n = raw.len();
    let mut at = 0;
    while at < n {
        if !is_hat(raw[at].trim_start()) {
            at += 1;
            continue;
        }
        let start = at;
        let mut depth = bracket_delta(&code[at]);
        let mut end = at;
        let mut probe = at + 1;
        while probe < n {
            let line = raw[probe].trim_start();
            if depth > 0 || is_hat(line) {
                depth += bracket_delta(&code[probe]);
                end = probe;
                probe += 1;
                continue;
            }
            break;
        }
        // A hat that owns nothing is prose, and prose gets no number — except at the file's own head, which is always worth a line a change can name.
        if start == 0 || (end + 1 < n && !raw[end + 1].trim().is_empty()) {
            table.span(start, end, None, None);
        }
        at = end + 1;
    }
}

// A long function's own control-flow blocks: `if`, `match`, `loop`, and the closures after a `=`.
fn control(code: &[String], ends: &[Option<usize>], table: &mut Table) {
    for (open, close) in scan::frames(code, '{', '}') {
        if head_of(code[open].trim_start()) != Some("fn") || close <= open + LONG {
            continue;
        }
        for (at, line) in code.iter().enumerate().take(close).skip(open + 1) {
            if !control_head(line.trim()) || table.key(at) {
                continue;
            }
            let end = ends.get(at).copied().flatten().unwrap_or(at);
            table.span(at, end, None, None);
        }
    }
}

// The head word of a declaration, once its modifiers are behind it; `None` when the line opens no piece. `pub async fn` → `fn`, `pub(crate) mod` → `mod`.
fn head_of(line: &str) -> Option<&'static str> {
    for raw in line.split_whitespace() {
        if raw.starts_with('#') || raw.starts_with('!') {
            continue;
        }
        let word: String = raw
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '!')
            .collect();
        if word.is_empty() {
            continue;
        }
        if let Some(head) = HEADS.iter().find(|head| **head == word) {
            return Some(head);
        }
        if MODIFIERS.contains(&word.as_str()) {
            continue;
        }
        return None;
    }
    None
}

// A header's ends: the line the body opens on, and whether there is a body at all. Brackets are counted, so a wrapped `where` clause and nested generics stay inside the header; `->` and `=>` contribute no closing angle.
fn span(code: &[String], start: usize) -> Option<(usize, bool)> {
    let mut paren = 0i64;
    let mut square = 0i64;
    let mut angle = 0i64;
    for (at, line) in code.iter().enumerate().skip(start) {
        let chars: Vec<char> = line.chars().collect();
        for (k, c) in chars.iter().enumerate() {
            match c {
                '(' => paren += 1,
                ')' => paren -= 1,
                '[' => square += 1,
                ']' => square -= 1,
                '<' => angle += 1,
                '>' => {
                    if !matches!(chars.get(k.wrapping_sub(1)), Some('-' | '=')) {
                        angle -= 1;
                    }
                }
                '{' if paren <= 0 && square <= 0 && angle <= 0 => return Some((at, true)),
                ';' if paren <= 0 && square <= 0 && angle <= 0 => return Some((at, false)),
                _ => {}
            }
        }
    }
    None
}

// A header's lines joined into one, so a wrapped signature reads as one line.
fn header_text(raw: &[&str], head: usize, open: usize) -> String {
    let mut text = String::new();
    for line in raw.iter().take(open + 1).skip(head) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(line);
    }
    text
}

// What a declaration is: the head template it reads by, and the line's own header with its body's opener dropped. `pub async fn send` → `pub async fn` and `pub async fn send`; `impl Display for Env` → `impl a for b` and `impl Display for Env`.
fn names_of(kind: &'static str, header: &str) -> (String, Option<String>) {
    let exact = without_body(header, kind);
    let mut label = words_before(&exact, kind);
    if !label.is_empty() {
        label.push(' ');
    }
    label.push_str(kind);
    if kind == "impl" && exact.contains(" for ") {
        label.push_str(" a for b");
    }
    (label, Some(exact))
}

// A header with its body's own punctuation behind it: `fn f() {}` → `fn f()`, `struct S;` → `struct S`, while a `use` tree keeps its braces.
fn without_body(header: &str, kind: &str) -> String {
    let mut exact = header.trim();
    if let Some(rest) = exact.strip_suffix(';') {
        exact = rest.trim_end();
    }
    // A `use` tree's braces are its own shape, not a body this piece opens.
    if kind == "use" {
        return exact.to_string();
    }
    if let Some(rest) = exact.strip_suffix("{}") {
        exact = rest.trim_end();
    } else if let Some(rest) = exact.strip_suffix('{') {
        exact = rest.trim_end();
    }
    exact.to_string()
}

// The modifier words standing before the head word, joined: `pub async unsafe fn` → `pub async unsafe`.
fn words_before(header: &str, kind: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for word in header.split_whitespace() {
        if raw_word(word) == kind {
            break;
        }
        out.push(word);
    }
    out.join(" ")
}

// A token's leading identifier: `pub(crate)` → `pub`, `impl<T>` → `impl`.
fn raw_word(token: &str) -> String {
    token
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '!')
        .collect()
}

// The kind of piece a line opens: the head word of its label, so `impl a for b` reads `impl`.
pub(crate) fn shape(line: &Line) -> Option<&str> {
    let label = line.label.as_deref()?;
    label.split_whitespace().find(|word| HEADS.contains(word))
}

fn is_hat(line: &str) -> bool {
    // Every comment counts, not only `///` and `//!`: a file's own head is a block like any other.
    line.starts_with("//") || line.starts_with("#[") || line.starts_with("#![")
}

fn bracket_delta(code: &str) -> i32 {
    let mut delta = 0;
    for ch in code.chars() {
        match ch {
            '[' => delta += 1,
            ']' => delta -= 1,
            _ => {}
        }
    }
    delta
}

// A line that opens a multi-line block inside a long function, in three passes: a keyword, the same keyword behind an expression prefix, then a macro call or a closure.
fn control_head(line: &str) -> bool {
    let line = line.trim();
    // A line may close the block above and open the next one: `} else if cond {`.
    let head = line.strip_prefix('}').map_or(line, str::trim_start);
    if control_word(head) {
        return true;
    }
    if control_word(expression_tail(head)) {
        return true;
    }
    // A macro or a closure counts only when its `{` ends the line: `.map(|x| x + 1)` is one line, and a folded `|` is not a block.
    line.ends_with('{') && (line.contains("!{") || line.contains("! {") || closure(line))
}

// The words that open a block, whole: `elsewhere()` and `loop_count` are not two of them.
fn control_word(line: &str) -> bool {
    if line.starts_with("if ") || line.starts_with("match ") || line.starts_with("while ") {
        return true;
    }
    if line.starts_with("for ") {
        return true;
    }
    for word in ["else", "loop", "unsafe", "async"] {
        if let Some(rest) = line.strip_prefix(word)
            && (rest.is_empty() || rest.starts_with(' ') || rest.starts_with('{'))
        {
            return true;
        }
    }
    false
}

// The expression a block may open behind: after `let x =`, after an assignment, after a jump.
fn expression_tail(line: &str) -> &str {
    // `let Some(x) = foo() else {` opens its block at the `else`, not at the call.
    if line.starts_with("let ") && line.trim_end().ends_with("else {") {
        return "else {";
    }
    if let Some(rest) = line.strip_prefix("return ") {
        return rest.trim_start();
    }
    if let Some(rest) = line.strip_prefix("break ") {
        return rest.trim_start();
    }
    match assignment(line) {
        Some(right) => right.trim_start(),
        None => "",
    }
}

// The right side of an assignment, and only an assignment: a comparison is not one.
fn assignment(line: &str) -> Option<&str> {
    let (left, right) = line.split_once('=')?;
    let left = left.trim_end();
    if right.starts_with('=') || left.ends_with(['=', '!', '<', '>']) {
        return None;
    }
    Some(right)
}

// `spawn(move || {` and `.map(|x| {`: the line opens with a closure's brace.
fn closure(line: &str) -> bool {
    line.strip_suffix('{')
        .map(str::trim_end)
        .is_some_and(|head| head.ends_with('|'))
}

// The Rust delimiters: comments, quoted and raw literals, char literals and their prefixes.
fn token(blank: &mut Blank, c: char) -> bool {
    match c {
        '/' if blank.peek(1) == Some('/') => {
            blank.skip(2);
            blank.skip_to_eol();
            blank.gap();
        }
        '/' if blank.peek(1) == Some('*') => {
            blank.skip(2);
            blank.skip_block(true);
            blank.gap();
        }
        '"' => {
            blank.skip(1);
            blank.gap();
            blank.skip_quote('"');
        }
        '\'' => return quote(blank),
        'r' | 'b' => return prefixed(blank, c),
        _ => return false,
    }
    true
}

// A char literal closes within a few characters; a lifetime never closes.
fn quote(blank: &mut Blank) -> bool {
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

// `r"…"`, `r#"…"#`, `b"…"`, `b'…'`, `br#"…"#`: the prefixes that keep a literal whole.
fn prefixed(blank: &mut Blank, c: char) -> bool {
    if c == 'r' {
        let mut k = 1;
        while blank.peek(k) == Some('#') {
            k += 1;
        }
        if blank.peek(k) == Some('"') {
            let hashes = k - 1;
            blank.skip(k + 1);
            blank.gap();
            blank.skip_raw(hashes);
            return true;
        }
    }
    if c == 'b' {
        match blank.peek(1) {
            Some('"') => {
                blank.skip(2);
                blank.gap();
                blank.skip_quote('"');
                return true;
            }
            Some('\'') => {
                blank.skip(1);
                return quote(blank);
            }
            Some('r') => {
                let mut k = 2;
                while blank.peek(k) == Some('#') {
                    k += 1;
                }
                if blank.peek(k) == Some('"') {
                    let hashes = k - 2;
                    blank.skip(k + 1);
                    blank.gap();
                    blank.skip_raw(hashes);
                    return true;
                }
            }
            _ => {}
        }
    }
    blank.hold(c);
    true
}
