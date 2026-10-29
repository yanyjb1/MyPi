//! Writing one key back into `config.yaml`, textually.

/// Write `value` at `keys`; every level is a direct child, everything else stays byte-for-byte.
pub fn write_key(path: &std::path::Path, keys: &[&str], value: &str) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<String> = text.split_inclusive('\n').map(str::to_string).collect();
    if lines.is_empty() {
        lines.push(String::new());
    }

    let mut found: Option<(usize, usize)> = None;
    let mut missing: Option<usize> = None;
    for (depth, key) in keys.iter().enumerate() {
        let (from, to) = match found {
            Some((line, _)) => (line + 1, block_end(&lines, line)),
            None => (0, lines.len()),
        };
        let hit = match found {
            None => (from..to).find(|&i| indent(&lines[i]) == 0 && opens_with(&lines[i], key)),
            Some((_, parent_indent)) => find_child(&lines, from, to, parent_indent, key),
        };
        match hit {
            Some(line) => found = Some((line, indent(&lines[line]))),
            None => {
                missing = Some(depth);
                break;
            }
        }
    }

    match missing {
        None => {
            let (leaf, leaf_indent) = found.expect("the last level was found");
            let comment = trailing_comment(&lines[leaf]).to_string();
            let nl = if lines[leaf].ends_with('\n') {
                "\n"
            } else {
                ""
            };
            lines[leaf] = format!(
                "{}{}: {value}{comment}{nl}",
                pad(leaf_indent),
                keys[keys.len() - 1]
            );
        }
        Some(depth) => {
            let (anchor, first_indent) = match found {
                Some((line, parent_indent)) => (Some(line), parent_indent + 2),
                None => (None, 0),
            };
            let last = keys.len() - 1;
            let chain: Vec<String> = keys[depth..]
                .iter()
                .enumerate()
                .map(|(k, key)| {
                    let at = first_indent + 2 * k;
                    if depth + k == last {
                        format!("{}{key}: {value}\n", pad(at))
                    } else {
                        format!("{}{key}:\n", pad(at))
                    }
                })
                .collect();
            match anchor {
                Some(at) => {
                    for (k, line) in chain.into_iter().enumerate() {
                        lines.insert(at + 1 + k, line);
                    }
                }
                None => {
                    if let Some(last_line) = lines.last_mut()
                        && !last_line.ends_with('\n')
                    {
                        last_line.push('\n');
                    }
                    lines.extend(chain);
                }
            }
        }
    }
    write_back(path, &lines)
}

fn write_back(path: &std::path::Path, lines: &[String]) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, lines.concat())?;
    Ok(())
}

fn opens_with(line: &str, key: &str) -> bool {
    let t = line.trim_start();
    t.starts_with(key) && t.as_bytes().get(key.len()) == Some(&b':')
}

fn child_indent(lines: &[String], from: usize, to: usize, parent: usize) -> Option<usize> {
    (from..to)
        .filter(|&i| {
            let t = lines[i].trim();
            !t.is_empty() && !t.starts_with('#') && indent(&lines[i]) > parent
        })
        .map(|i| indent(&lines[i]))
        .min()
}

fn find_child(lines: &[String], from: usize, to: usize, parent: usize, key: &str) -> Option<usize> {
    let at = child_indent(lines, from, to, parent)?;
    (from..to).find(|&i| indent(&lines[i]) == at && opens_with(&lines[i], key))
}

fn block_end(lines: &[String], at: usize) -> usize {
    let base = indent(&lines[at]);
    let mut i = at + 1;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || indent(&lines[i]) > base {
            i += 1;
        } else {
            break;
        }
    }
    i
}

fn indent(line: &str) -> usize {
    line.len() - line.trim_start().len()
}

fn pad(n: usize) -> String {
    " ".repeat(n)
}

fn trailing_comment(line: &str) -> &str {
    let Some(at) = comment_start(line) else {
        return "";
    };
    let end = line.trim_end_matches('\n').len();
    let start = line[..at]
        .rfind(|c: char| !c.is_whitespace())
        .map(|i| i + 1)
        .unwrap_or(0);
    &line[start..end]
}

fn comment_start(line: &str) -> Option<usize> {
    let b = line.as_bytes();
    let mut quote: Option<u8> = None;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match quote {
            Some(q) if c == b'\\' && q == b'"' => i += 1,
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == b'\'' || c == b'"' => quote = Some(c),
            None if c == b'#' && (i == 0 || b[i - 1].is_ascii_whitespace()) => return Some(i),
            None => {}
        }
        i += 1;
    }
    None
}
