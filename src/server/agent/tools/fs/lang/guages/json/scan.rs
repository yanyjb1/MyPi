// The JSON scan: which line each container opens and closes on, read off the brackets with string bodies blanked first.

// One container of a document: the line it opens on, the line it closes on, and the key that names it when one does.
pub(crate) struct Shape {
    // 1-based.
    pub(crate) open: usize,

    // 1-based, equal to `open` when the whole container stands on one line.
    pub(crate) close: usize,

    // The key standing before the opening bracket, without its quotes; `None` for an unnamed element of a list.
    pub(crate) key: Option<String>,
}

// One bracket still open while the lines are read.
struct Open {
    line: usize,
    key: Option<String>,
}

// Every container of one document, in the order its brackets close. A document whose brackets do not pair answers nothing: its own parser has already refused it.
pub(crate) fn shapes(text: &str) -> Vec<Shape> {
    let mut out: Vec<Shape> = Vec::new();
    let mut stack: Vec<Open> = Vec::new();
    for (at, line) in text.lines().enumerate() {
        let raw: Vec<char> = line.chars().collect();
        let scan = blanked(&raw);
        for (col, one) in scan.iter().enumerate() {
            match one {
                '{' | '[' => stack.push(Open {
                    line: at,
                    key: key(&raw[..col]),
                }),
                '}' | ']' => {
                    let Some(held) = stack.pop() else {
                        return Vec::new();
                    };
                    out.push(Shape {
                        open: held.line + 1,
                        close: at + 1,
                        key: held.key,
                    });
                }
                _ => {}
            }
        }
    }
    if stack.is_empty() { out } else { Vec::new() }
}

// The key a bracket is introduced by: the last one named before it on its own line, so `{"a": 1, "b": {` names `b`.
fn key(head: &[char]) -> Option<String> {
    let head: String = head.iter().collect();
    let (before, _) = head.rsplit_once(':')?;
    let before = before
        .rsplit([',', '{', '['])
        .next()
        .unwrap_or(before)
        .trim();
    let key = before.strip_prefix('"')?.strip_suffix('"')?;
    (!key.is_empty()).then(|| key.to_string())
}

// One line with every string body turned to spaces, so a bracket inside a value opens nothing; the quotes themselves stay, so a key still reads as one.
fn blanked(raw: &[char]) -> Vec<char> {
    let mut out = Vec::with_capacity(raw.len());
    let mut quoted = false;
    let mut escaped = false;
    for &one in raw {
        if !quoted {
            quoted = one == '"';
            out.push(one);
            continue;
        }
        if escaped {
            escaped = false;
            out.push(' ');
        } else if one == '\\' {
            escaped = true;
            out.push(' ');
        } else if one == '"' {
            quoted = false;
            out.push(one);
        } else {
            out.push(' ');
        }
    }
    out
}
