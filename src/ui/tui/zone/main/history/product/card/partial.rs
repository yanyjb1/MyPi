//! Reading tool arguments while they are still arriving.

/// The first string value of this key, complete or cut short by the stream.
pub(crate) fn text(args: &str, key: &str) -> Option<String> {
    let bytes = args.as_bytes();
    let mut at = 0usize;
    while at < bytes.len() {
        if bytes[at] != b'"' {
            at += 1;
            continue;
        }
        let (name, end, closed) = string_at(args, at)?;
        at = end;
        if !closed {
            return None;
        }
        let Some(after) = skip(bytes, end) else {
            continue;
        };
        if bytes[after] != b':' {
            continue;
        }
        let Some(value) = skip(bytes, after + 1) else {
            continue;
        };
        if bytes[value] != b'"' {
            at = value;
            continue;
        }
        let value = string_at(args, value)?;
        if name == key {
            return Some(value.0);
        }
        at = value.1;
    }
    None
}

/// Every `(number, text)` pair of this object key, in the order the pairs arrive.
pub(crate) fn records(args: &str, num: &str, text: &str) -> Vec<(Option<usize>, String)> {
    let bytes = args.as_bytes();
    let mut out: Vec<(Option<usize>, String)> = Vec::new();
    let mut current: Option<usize> = None;
    let mut at = 0usize;
    while at < bytes.len() {
        if bytes[at] != b'"' {
            at += 1;
            continue;
        }
        let Some((name, end, closed)) = string_at(args, at) else {
            break;
        };
        at = end;
        let Some(after) = skip(bytes, end).filter(|_| closed) else {
            break;
        };
        if bytes[after] != b':' {
            continue;
        }
        let Some(value) = skip(bytes, after + 1) else {
            break;
        };
        if name == num {
            let (number, end) = digits(bytes, value);
            if let Some(n) = number {
                current = Some(n);
            }
            at = end;
            continue;
        }
        if name == text && bytes[value] == b'"' {
            match string_at(args, value) {
                Some((body, end, _)) => {
                    out.push((current.take(), body));
                    at = end;
                    continue;
                }
                None => break,
            }
        }
        at = value;
    }
    out
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
                let next = *bytes.get(at + 1)?;
                let simple = match next {
                    b'"' => Some('"'),
                    b'\\' => Some('\\'),
                    b'/' => Some('/'),
                    b'b' => Some('\u{8}'),
                    b'f' => Some('\u{c}'),
                    b'n' => Some('\n'),
                    b'r' => Some('\r'),
                    b't' => Some('\t'),
                    _ => None,
                };
                if let Some(ch) = simple {
                    out.push(ch);
                    at += 2;
                    continue;
                }
                if next != b'u' {
                    out.push(next as char);
                    at += 2;
                    continue;
                }
                let mut code = u32::from_str_radix(text.get(at + 2..at + 6)?, 16).ok()?;
                let mut width = 6usize;
                if (0xD800..=0xDBFF).contains(&code) {
                    let low = u32::from_str_radix(text.get(at + 8..at + 12)?, 16).ok()?;
                    if !(0xDC00..=0xDFFF).contains(&low) {
                        return None;
                    }
                    code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                    width = 12;
                }
                out.push(char::from_u32(code)?);
                at += width;
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

fn skip(bytes: &[u8], from: usize) -> Option<usize> {
    let mut at = from;
    while at < bytes.len() && bytes[at].is_ascii_whitespace() {
        at += 1;
    }
    (at < bytes.len()).then_some(at)
}

fn digits(bytes: &[u8], from: usize) -> (Option<usize>, usize) {
    let mut at = from;
    let mut out = 0usize;
    while at < bytes.len() && bytes[at].is_ascii_digit() {
        out = out * 10 + usize::from(bytes[at] - b'0');
        at += 1;
    }
    ((at > from).then_some(out), at)
}
