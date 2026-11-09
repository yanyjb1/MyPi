//! Command segments: where one command ends and the next begins, quotes and escapes respected.

/// Shell separators; a word after one starts a new command segment. A bare newline counts.
pub(crate) fn is_sep(c: char) -> bool {
    c == ';' || c == '&' || c == '|' || c == '(' || c == ')' || c == '`' || c == '\n' || c == '\r'
}

/// Byte ranges of the words that start a command segment, in order.
pub(crate) fn heads(cmd: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut chars = cmd.char_indices().peekable();
    let mut at_head = true;
    while let Some((i, c)) = chars.next() {
        match c {
            '\'' | '"' => {
                let quote = c;
                while let Some((_, d)) = chars.next() {
                    if d == '\\' && quote == '"' {
                        chars.next();
                        continue;
                    }
                    if d == quote {
                        break;
                    }
                }
                at_head = false;
            }
            '\\' => match chars.next() {
                // A backslash before a newline continues the line: not a separator, not a word.
                Some((_, '\n')) | Some((_, '\r')) => {}
                Some(_) => at_head = false,
                None => {}
            },
            _ if is_sep(c) => at_head = true,
            _ if c.is_whitespace() => {}
            _ => {
                if at_head {
                    let mut end = i + c.len_utf8();
                    while let Some(&(j, d)) = chars.peek() {
                        if d.is_whitespace() || is_sep(d) || matches!(d, '\'' | '"' | '\\') {
                            break;
                        }
                        chars.next();
                        end = j + d.len_utf8();
                    }
                    out.push((i, end));
                    at_head = false;
                }
            }
        }
    }
    out
}
