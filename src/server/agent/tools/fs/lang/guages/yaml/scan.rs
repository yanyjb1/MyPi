// The YAML scan: where a block starts, how far its own indented body reaches, and what the block is called.

// The shape of one line, as indentation and its first token read it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    // Nothing to own: a blank line, a comment, a document marker, a directive.
    Idle,

    // A mapping key, with or without a value on its own line.
    Key,

    // A `- ` item, with or without a key of its own.
    Item,
}

// How a line reads: its indentation in spaces, and what it opens. A tab counts as eight, which no YAML document may use for indentation but which keeps a broken line's shape legible.
pub(crate) fn read(line: &str) -> (usize, Kind) {
    let indent = line
        .chars()
        .take_while(|one| *one == ' ' || *one == '\t')
        .map(|one| if one == '\t' { 8 } else { 1 })
        .sum();
    let text = line.trim_start();
    if text.is_empty()
        || text.starts_with('#')
        || text.starts_with("---")
        || text.starts_with("...")
    {
        return (indent, Kind::Idle);
    }
    if text.starts_with("%") {
        return (indent, Kind::Idle);
    }
    (indent, if dash(text) { Kind::Item } else { Kind::Key })
}

// Whether this text opens a `- ` item: a dash, then a space or the line's end.
fn dash(text: &str) -> bool {
    match text.strip_prefix('-') {
        Some(rest) => rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t'),
        None => false,
    }
}

// The name a block carries: a key without its colon, or an item's own key when it opens with one. `None` for a bare item and for a key that names nothing.
pub(crate) fn name(line: &str) -> Option<String> {
    let text = line.trim_start();
    let text = if dash(text) {
        text[1..].trim_start()
    } else {
        text
    };
    let (key, _) = split_key(text)?;
    let key = key.trim();
    (!key.is_empty()).then(|| key.to_string())
}

// What a key line opens below it.
pub(crate) enum Opens {
    // A `|` or `>` marker: the body below is a string, not structure.
    Scalar,

    // An empty value: the lines below are a nested mapping or sequence.
    Nested,

    // The value stands on its own line, so nothing is nested.
    No,
}

// What a key line opens below it: a block scalar's marker, an empty value, or a value of its own.
pub(crate) fn opens(line: &str) -> Opens {
    let text = line.trim_start();
    let text = if dash(text) {
        text[1..].trim_start()
    } else {
        text
    };
    // A bare marker line — `- |`, an item facing nothing but a scalar — opens a scalar with no key of its own.
    let Some((_, value)) = split_key(text) else {
        return if marker(text) {
            Opens::Scalar
        } else {
            Opens::No
        };
    };
    let value = value.trim();
    if marker(value) {
        Opens::Scalar
    } else if value.is_empty() {
        Opens::Nested
    } else {
        Opens::No
    }
}

// Whether this text is a block scalar's marker: `|` or `>`, then an optional chomping or indentation indicator.
fn marker(text: &str) -> bool {
    let Some(rest) = text.strip_prefix(['|', '>']) else {
        return false;
    };
    rest.chars()
        .all(|one| matches!(one, '-' | '+') || one.is_ascii_digit())
}

// The key and the value of one `key: value` line, split at the first colon that ends a key. A colon inside a quoted key, a flow collection or a URL's own body does not end it.
fn split_key(text: &str) -> Option<(&str, &str)> {
    let mut quoted: Option<char> = None;
    let mut flow = 0usize;
    let mut escaped = false;
    for (at, one) in text.char_indices() {
        match quoted {
            Some(quote) => {
                if escaped {
                    escaped = false;
                } else if one == '\\' && quote == '"' {
                    escaped = true;
                } else if one == quote {
                    quoted = None;
                }
            }
            None => match one {
                '"' | '\'' => quoted = Some(one),
                '[' | '{' => flow += 1,
                ']' | '}' => flow = flow.saturating_sub(1),
                ':' if flow == 0 => {
                    let after = &text[at + 1..];
                    if after.is_empty() || after.starts_with(' ') || after.starts_with('\t') {
                        return Some((&text[..at], after));
                    }
                }
                _ => {}
            },
        }
    }
    None
}

// How far a block opened at `open` reaches: while lines stay idle, or indented deeper than the opener, and the last line of that run that is not idle. A block whose body is indented no deeper than its opener owns only its own line.
pub(crate) fn extent(raw: &[&str], open: usize, indent: usize) -> usize {
    let mut close = open;
    let mut at = open + 1;
    while at < raw.len() {
        let (depth, kind) = read(raw[at]);
        if kind == Kind::Idle {
            at += 1;
            continue;
        }
        if depth <= indent {
            break;
        }
        close = at;
        at += 1;
    }
    close
}

// How far a block scalar's body runs from its marker line: every following line indented deeper than the marker's own line, blank lines included, up to the first line that is not. A sibling item or key at the marker's depth ends the body.
pub(crate) fn scalar(raw: &[&str], open: usize, indent: usize) -> usize {
    let mut at = open + 1;
    while at < raw.len() && (raw[at].trim().is_empty() || indent_of(raw[at]) > indent) {
        at += 1;
    }
    at.saturating_sub(1)
}

// How far one line is indented, a tab counting as eight.
fn indent_of(line: &str) -> usize {
    line.chars()
        .take_while(|one| *one == ' ' || *one == '\t')
        .map(|one| if one == '\t' { 8 } else { 1 })
        .sum()
}
