// The XML scan: which line each element opens on and which line closes it, read off the tags with comments, CDATA and declarations stepped over.

// One element of a document: the line its start tag opens on, the line its end tag closes on, and what a splice anchor copies.
pub(crate) struct Shape {
    // 1-based.
    pub(crate) open: usize,

    // 1-based, equal to `open` when the element closes on the line it opened on, or is self-closing.
    pub(crate) close: usize,

    // The tag, and its `id` or `name` when it carries one, so two elements of one tag can be told apart.
    pub(crate) exact: String,
}

// One start tag still open while the document is read.
struct Open {
    open: usize,
    exact: String,
}

// Every element of one document, in the order its start tags appear. A document whose tags do not pair answers nothing: its own parser has already refused it.
pub(crate) fn shapes(text: &str) -> Vec<Shape> {
    let mut out: Vec<Shape> = Vec::new();
    let mut stack: Vec<Open> = Vec::new();
    let mut line = 1usize;
    let mut at = 0usize;
    while at < text.len() {
        let rest = &text[at..];
        let next = if rest.starts_with("<!--") {
            moved(text, at, "-->", 4)
        } else if rest.starts_with("<![CDATA[") {
            moved(text, at, "]]>", 9)
        } else if rest.starts_with("<?") || rest.starts_with("<!") {
            let Some(end) = ended(text, at) else {
                return Vec::new();
            };
            end
        } else if rest.starts_with('<') {
            let Some(end) = ended(text, at) else {
                return Vec::new();
            };
            let tag = text[at + 1..end - 1].trim();
            if let Some(name) = tag.strip_prefix('/') {
                let Some(held) = stack.pop() else {
                    return Vec::new();
                };
                if held.exact.split('#').next() != Some(name.trim()) {
                    return Vec::new();
                }
                out.push(Shape {
                    open: held.open,
                    close: line,
                    exact: held.exact,
                });
            } else {
                let closed = tag.ends_with('/');
                let body = tag.trim_end_matches('/').trim();
                let name: String = body
                    .chars()
                    .take_while(|one| !one.is_whitespace())
                    .collect();
                if !name.is_empty() {
                    let exact = anchor(&body[name.len()..], &name);
                    if closed {
                        out.push(Shape {
                            open: line,
                            close: line,
                            exact,
                        });
                    } else {
                        stack.push(Open { open: line, exact });
                    }
                }
            }
            end
        } else {
            at + rest.chars().next().map_or(1, char::len_utf8)
        };
        // The run just stepped over is the only text whose newlines move the count, so one pass counts them all.
        line += text[at..next].matches('\n').count();
        at = next;
    }
    if stack.is_empty() { out } else { Vec::new() }
}

// Past a run that ends at `token`, which starts `opened` bytes in; the text's own end when it never closes.
fn moved(text: &str, at: usize, token: &str, opened: usize) -> usize {
    text[at + opened..]
        .find(token)
        .map_or(text.len(), |end| at + opened + end + token.len())
}

// The byte just past the `>` closing the tag that opens at `at`. A `>` inside a quoted attribute value is that value's own character, and `None` means the tag never ends.
fn ended(text: &str, at: usize) -> Option<usize> {
    let mut quoted: Option<char> = None;
    for (offset, one) in text[at + 1..].char_indices() {
        match quoted {
            Some(quote) => {
                if one == quote {
                    quoted = None;
                }
            }
            None => match one {
                '"' | '\'' => quoted = Some(one),
                '>' => return Some(at + offset + 2),
                _ => {}
            },
        }
    }
    None
}

// What an element answers to: its tag, and the `id` or `name` it carries when it carries one.
fn anchor(attrs: &str, name: &str) -> String {
    for pair in attrs.split_whitespace() {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if key != "id" && key != "name" {
            continue;
        }
        let value = value.trim_matches(['"', '\'']);
        if !value.is_empty() {
            return format!("{name}#{value}");
        }
    }
    name.to_string()
}
