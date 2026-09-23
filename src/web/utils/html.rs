//! Shared HTML extraction helpers — regex-level scraping utilities used by
//! both engine parsers (Bing `li.b_algo`, DDG `result__a`). Flat, stable
//! markup only; a DOM parser would be heavier than this crate needs.

pub(crate) fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for ch in s.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    // Collapse whitespace runs (titles embed <strong> markers between words).
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn unescape_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

// Decode Bing's click-tracking wrapper: /ck/a?…&u=a1<base64url-target>
// (and the older &u={target} shape). Returns the input unchanged when the
// href is a plain link.
pub(crate) struct Tag {
    pub(crate) open_tag: String,
    pub(crate) inner: String,
}

pub(crate) fn extract_between(hay: &str, open_prefix: &str, close: &str) -> Option<Tag> {
    let start = hay.find(open_prefix)?;
    let gt = hay[start..].find('>')? + start;
    let open_tag = hay[start..=gt].to_string();
    let inner_start = gt + 1;
    let end = hay[inner_start..].find(close)? + inner_start;
    Some(Tag { open_tag, inner: hay[inner_start..end].to_string() })
}

pub(crate) fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    // href="…" or href='…'
    for q in ['"', '\''] {
        let pat = format!("{attr}={q}");
        if let Some(pos) = tag.find(&pat) {
            let rest = &tag[pos + pat.len()..];
            let end = rest.find(q)?;
            return Some(rest[..end].to_string());
        }
    }
    None
}

