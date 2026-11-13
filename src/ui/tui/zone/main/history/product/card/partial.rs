//! Reading tool arguments while they are still arriving.

use serde_json::Value;

/// The first string value of this key, complete or cut short by the stream.
pub(crate) fn text(args: &str, key: &str) -> Option<String> {
    let mut at = args.find('{')? + 1;
    while let Some((name, value)) = next_pair(args, at) {
        if name == key {
            return match args.as_bytes().get(value) {
                Some(b'"') => string_at(args, value).map(|(body, _, _)| body),
                _ => None,
            };
        }
        // An earlier value that is still arriving hides what follows it.
        at = skip_value(args, value)?;
    }
    None
}

/// The elements of the array under this key that have arrived whole: objects and strings both.
pub(crate) fn items(args: &str, key: &str) -> Vec<Value> {
    let Some(bracket) = array_of(args, key) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut at = bracket + 1;
    loop {
        let Some(offset) = args[at..].find(|c: char| !c.is_whitespace() && c != ',') else {
            break;
        };
        let start = at + offset;
        match args.as_bytes()[start] {
            b'{' => {
                let Some(end) = balanced(args, start) else {
                    break;
                };
                if let Ok(value) = serde_json::from_str::<Value>(&args[start..end]) {
                    out.push(value);
                }
                at = end;
            }
            b'"' => {
                let Some((body, end, closed)) = string_at(args, start) else {
                    break;
                };
                if !closed {
                    break;
                }
                out.push(Value::String(body));
                at = end;
            }
            _ => break,
        }
    }
    out
}

/// The byte just past the `[` of the array under this key.
fn array_of(args: &str, key: &str) -> Option<usize> {
    let mut at = args.find('{')? + 1;
    loop {
        let (name, value) = next_pair(args, at)?;
        if name == key {
            return (args.as_bytes().get(value) == Some(&b'[')).then_some(value);
        }
        at = skip_value(args, value)?;
    }
}

/// The byte just past one whole value; `None` while any of it is still arriving.
fn skip_value(args: &str, at: usize) -> Option<usize> {
    match args.as_bytes().get(at)? {
        b'"' => string_at(args, at)
            .filter(|(_, _, closed)| *closed)
            .map(|(_, end, _)| end),
        b'{' | b'[' => balanced(args, at),
        _ => {
            let rest = &args[at..];
            let end = rest.find([',', '}', ']'])?;
            Some(at + end)
        }
    }
}

/// The byte just past the bracket that closes the one at `at`; `None` while it is still arriving.
fn balanced(args: &str, at: usize) -> Option<usize> {
    let bytes = args.as_bytes();
    if !matches!(bytes.get(at), Some(b'{' | b'[')) {
        return None;
    }
    let mut depth = 0usize;
    let mut index = at;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                let (_, end, closed) = string_at(args, index)?;
                if !closed {
                    return None;
                }
                index = end;
            }
            b'{' | b'[' => {
                depth += 1;
                index += 1;
            }
            b'}' | b']' => {
                depth -= 1;
                index += 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => index += 1,
        }
    }
    None
}

/// One string field of an item that has already arrived.
pub(crate) fn field(item: &Value, key: &str) -> Option<String> {
    item.get(key)?.as_str().map(str::to_string)
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
