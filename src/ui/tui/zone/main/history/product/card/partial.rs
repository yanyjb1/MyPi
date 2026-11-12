//! Reading tool arguments while they are still arriving.

/// The first string value of this key, complete or cut short by the stream.
pub(crate) fn text(args: &str, key: &str) -> Option<String> {
    let mut at = args.find('{')? + 1;
    while let Some((name, value)) = next_pair(args, at) {
        if name == key && args.as_bytes().get(value) == Some(&b'"') {
            return string_at(args, value).map(|(body, _, _)| body);
        }
        at = value_end(args, value);
    }
    None
}

/// The next key and the byte its value starts at, at or after `from`.
fn next_pair(args: &str, from: usize) -> Option<(String, usize)> {
    let bytes = args.as_bytes();
    let mut at = from;
    while at < bytes.len() {
        if bytes[at] != b'"' {
            at += 1;
            continue;
        }
        let (name, end, closed) = string_at(args, at)?;
        if !closed {
            return None;
        }
        at = end;
        let colon = skip(bytes, end)?;
        if bytes[colon] != b':' {
            continue;
        }
        return Some((name, skip(bytes, colon + 1)?));
    }
    None
}

/// Where the value starting at `at` ends; a non-string keeps its own position.
fn value_end(args: &str, at: usize) -> usize {
    if args.as_bytes().get(at) != Some(&b'"') {
        return at;
    }
    string_at(args, at).map_or(args.len(), |(_, end, _)| end)
}

/// The decoded string that starts at this quote, and where it ends; `closed` is false at input end.
fn string_at(text: &str, quote: usize) -> Option<(String, usize, bool)> {
    let bytes = text.as_bytes();
    let mut out = String::new();
    let mut at = quote + 1;
    while at < bytes.len() {
        match bytes[at] {
            b'"' => return Some((out, at + 1, true)),
            b'\\' => {
                let Some(&next) = bytes.get(at + 1) else {
                    return Some((out, at, false));
                };
                if next == b'u' {
                    let mut code = hex4(text, at + 2)?;
                    let mut width = 6usize;
                    if (0xD800..=0xDBFF).contains(&code) {
                        let low = hex4(text, at + 8)?;
                        if !(0xDC00..=0xDFFF).contains(&low) {
                            return None;
                        }
                        code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                        width = 12;
                    }
                    out.push(char::from_u32(code)?);
                    at += width;
                    continue;
                }
                let simple = match next {
                    b'"' => '"',
                    b'\\' => '\\',
                    b'/' => '/',
                    b'b' => '\u{8}',
                    b'f' => '\u{c}',
                    b'n' => '\n',
                    b'r' => '\r',
                    b't' => '\t',
                    _ => return None,
                };
                out.push(simple);
                at += 2;
            }
            _ => {
                let ch = text[at..].chars().next()?;
                out.push(ch);
                at += ch.len_utf8();
            }
        }
    }
    Some((out, at, false))
}

/// The four hex digits of one escape, when they are all there.
fn hex4(text: &str, from: usize) -> Option<u32> {
    u32::from_str_radix(text.get(from..from + 4)?, 16).ok()
}

fn skip(bytes: &[u8], from: usize) -> Option<usize> {
    let mut at = from;
    while at < bytes.len() && bytes[at].is_ascii_whitespace() {
        at += 1;
    }
    (at < bytes.len()).then_some(at)
}
