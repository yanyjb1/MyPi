// HTML: its own tag scanner, and the sorter that reads elements, their ids and the scripts and styles they carry.

use super::LANGUAGE;
use crate::server::agent::tools::fs::lang::Table;

// Elements that own a piece of the page: their opening tag and their closing tag pair into one span.
const PIECES: &[&str] = &[
    "article", "aside", "body", "dialog", "footer", "form", "head", "header", "html", "main",
    "nav", "script", "section", "style", "template",
];

// Elements that never close.
const VOID: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

// Elements whose content is text, never markup: nothing inside them is read as a tag.
const RAW: &[&str] = &["script", "style"];

// One tag the scan met.
pub(crate) struct Tag {
    // The 0-based line its `<` stands on.
    pub(crate) line: usize,

    // The 0-based line its `>` stands on, when the tag wraps.
    pub(crate) end: usize,
    pub(crate) name: String,
    pub(crate) close: bool,

    // A tag that carries no closing of its own: `<br>`, `<img …>`, `<x />`.
    pub(crate) single: bool,

    // The `id` attribute's value, when the tag writes one.
    pub(crate) id: Option<String>,

    // The `class` attribute's value, when the tag writes one.
    pub(crate) class: Option<String>,

    // The `src` or `href` attribute's value, when the tag writes one.
    pub(crate) href: Option<String>,

    // The `rel` attribute's value, when the tag writes one.
    pub(crate) rel: Option<String>,
}

// Every tag of one text, in line order.
pub(crate) fn scan(text: &str) -> Vec<Tag> {
    let chars: Vec<char> = text.chars().collect();
    let mut at = 0usize;
    let mut line = 0usize;
    let mut out: Vec<Tag> = Vec::new();
    while at < chars.len() {
        match chars[at] {
            '\n' => {
                line += 1;
                at += 1;
            }
            '<' => {
                let start = line;
                if starts(&chars, at, "<!--") {
                    let stop = seek(&chars, at + 4, "-->");
                    line += chars[at..stop.min(chars.len())]
                        .iter()
                        .filter(|c| **c == '\n')
                        .count();
                    at = stop;
                    continue;
                }
                if matches!(chars.get(at + 1), Some('!' | '?')) {
                    let stop = seek(&chars, at + 2, ">");
                    line += chars[at..stop.min(chars.len())]
                        .iter()
                        .filter(|c| **c == '\n')
                        .count();
                    at = stop;
                    continue;
                }
                let close = chars.get(at + 1) == Some(&'/');
                let mut k = if close { at + 2 } else { at + 1 };
                let mut name = String::new();
                while let Some(c) = chars.get(k)
                    && (c.is_ascii_alphanumeric() || matches!(c, '-' | ':' | '_'))
                {
                    name.push(*c);
                    k += 1;
                }
                if name.is_empty() {
                    at += 1;
                    continue;
                }
                let attrs = k;
                let mut single = false;
                while k < chars.len() && chars[k] != '>' {
                    match chars[k] {
                        '\n' => {
                            line += 1;
                            k += 1;
                        }
                        '"' | '\'' => k = skip_quoted(&chars, k),
                        '/' if chars.get(k + 1) == Some(&'>') => {
                            single = true;
                            k += 1;
                        }
                        _ => k += 1,
                    }
                }
                let attr: String = chars[attrs..k.min(chars.len())].iter().collect();
                let end = line;
                at = (k + 1).min(chars.len());
                let lower = name.to_ascii_lowercase();
                let void = VOID.contains(&lower.as_str());
                out.push(Tag {
                    line: start,
                    end,
                    id: attribute(&attr, "id"),
                    class: attribute(&attr, "class"),
                    href: attribute(&attr, "href").or_else(|| attribute(&attr, "src")),
                    rel: attribute(&attr, "rel"),
                    single: single || void || close,
                    close,
                    name: lower.clone(),
                });
                if !close && RAW.contains(&lower.as_str()) {
                    let stop = format!("</{lower}");
                    let found = find_ci(&chars, at, &stop);
                    line += chars[at..found.min(chars.len())]
                        .iter()
                        .filter(|c| **c == '\n')
                        .count();
                    at = found;
                }
            }
            _ => at += 1,
        }
    }
    out
}

// Whether the text at `at` opens with `want`.
fn starts(chars: &[char], at: usize, want: &str) -> bool {
    let want: Vec<char> = want.chars().collect();
    chars.len() >= at + want.len() && chars[at..at + want.len()] == want[..]
}

// The place just past `want`, found at or after `from`.
fn seek(chars: &[char], from: usize, want: &str) -> usize {
    let want: Vec<char> = want.chars().collect();
    let mut at = from;
    while at + want.len() <= chars.len() {
        if chars[at..at + want.len()] == want[..] {
            return at + want.len();
        }
        at += 1;
    }
    chars.len()
}

// The place `want` stands at or after `from`, matched without regard to case; the text's end when it stands nowhere.
fn find_ci(chars: &[char], from: usize, want: &str) -> usize {
    let want: Vec<char> = want.chars().flat_map(char::to_lowercase).collect();
    let mut at = from;
    while at + want.len() <= chars.len() {
        let here: Vec<char> = chars[at..at + want.len()]
            .iter()
            .flat_map(|c| c.to_lowercase())
            .collect();
        if here == want {
            return at;
        }
        at += 1;
    }
    chars.len()
}

// The place just past a quoted attribute value the scan meets at `at`.
fn skip_quoted(chars: &[char], at: usize) -> usize {
    let mark = chars[at];
    let mut k = at + 1;
    while k < chars.len() {
        match chars[k] {
            c if c == mark => return k + 1,
            _ => k += 1,
        }
    }
    chars.len()
}

// One attribute's value, written either quoted or bare.
fn attribute(attr: &str, want: &str) -> Option<String> {
    let mut rest = attr;
    loop {
        let at = rest.find(want)?;
        let before = rest[..at].chars().next_back();
        let after = rest[at + want.len()..].trim_start();
        let bound = before.is_none_or(|c| c.is_whitespace());
        if !bound {
            rest = &rest[at + want.len()..];
            continue;
        }
        let Some(value) = after.strip_prefix('=') else {
            rest = &rest[at + want.len()..];
            continue;
        };
        let value = value.trim_start();
        let text = match value.chars().next() {
            Some(mark @ ('"' | '\'')) => {
                let inner = &value[mark.len_utf8()..];
                let end = inner.find(mark).unwrap_or(inner.len());
                &inner[..end]
            }
            _ => {
                let end = value
                    .find(|c: char| c.is_whitespace() || c == '>')
                    .unwrap_or(value.len());
                &value[..end]
            }
        };
        return Some(text.to_string()).filter(|text| !text.is_empty());
    }
}

// One file's line table: the elements that own a piece of the page, the id-carrying tags, and the folded pairs.
pub(crate) fn table(text: &str) -> Table {
    let tags = scan(text);
    let mut table = Table::new(text.lines().count(), &LANGUAGE);
    let mut stack: Vec<usize> = Vec::new();
    for (index, tag) in tags.iter().enumerate() {
        if tag.close {
            let found = stack.iter().rposition(|at| tags[*at].name == tag.name);
            let Some(found) = found else {
                continue;
            };
            let open = stack.remove(found);
            let head = &tags[open];
            if PIECES.contains(&head.name.as_str()) {
                let exact = format!("<{}>", head.name);
                table.span(head.line, tag.end, Some(head.name.clone()), Some(exact));
            }
            continue;
        }
        if tag.single {
            if PIECES.contains(&tag.name.as_str()) {
                let exact = format!("<{} />", tag.name);
                table.span(tag.line, tag.end, Some(tag.name.clone()), Some(exact));
            }
            continue;
        }
        stack.push(index);
    }
    for open in stack {
        let head = &tags[open];
        if PIECES.contains(&head.name.as_str()) {
            let exact = format!("<{}>", head.name);
            table.span(head.line, head.end, Some(head.name.clone()), Some(exact));
        }
    }
    // A tag that carries an id keys its line, unless a piece already starts there.
    for tag in &tags {
        if let Some(id) = &tag.id
            && !table.key(tag.line)
        {
            let exact = format!("#{id}");
            table.span(tag.line, tag.end, Some("id".to_string()), Some(exact));
        }
    }
    table
}

// The ids and classes this page names, with the line each stands on.
pub(super) fn anchors(tags: &[Tag]) -> Vec<(char, String, usize)> {
    let mut out: Vec<(char, String, usize)> = Vec::new();
    for tag in tags {
        if let Some(id) = &tag.id
            && !out.contains(&('#', id.clone(), tag.line))
        {
            out.push(('#', id.clone(), tag.line));
        }
        if let Some(class) = &tag.class {
            for name in class.split_whitespace() {
                if !out.contains(&('.', name.to_string(), tag.line)) {
                    out.push(('.', name.to_string(), tag.line));
                }
            }
        }
    }
    out
}

// Every stylesheet and script this page pulls in, as written, with the line it stands on.
pub(super) fn refs(tags: &[Tag]) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::new();
    for tag in tags {
        if tag.close {
            continue;
        }
        let Some(href) = &tag.href else {
            continue;
        };
        match tag.name.as_str() {
            "script" => {}
            "link" => {
                let rel = tag.rel.as_deref().unwrap_or("");
                if !rel.split_whitespace().any(|word| word == "stylesheet") {
                    continue;
                }
            }
            _ => continue,
        }
        out.push((tag.line, href.clone()));
    }
    out
}

// The elements whose content is code rather than markup: the line their tag ends on, the line their close stands on, and their own word.
pub(super) fn blocks(tags: &[Tag]) -> Vec<(usize, usize, &'static str)> {
    let mut out = Vec::new();
    for (at, tag) in tags.iter().enumerate() {
        if tag.close {
            continue;
        }
        let Some(word) = RAW.iter().copied().find(|word| tag.name.as_str() == *word) else {
            continue;
        };
        let close = tags[at + 1..]
            .iter()
            .find(|next| next.close && next.name == tag.name);
        if let Some(close) = close {
            out.push((tag.end, close.line, word));
        }
    }
    out
}
