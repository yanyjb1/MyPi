//! Shared HTML extraction helpers — tag-level scraping utilities for the
//! search-engine parsers (Bing `li.b_algo`, DDG `div.result`, Google's `h3`
//! anchors, …). Flat, stable markup only; a DOM parser would be a heavier
//! dependency than this crate wants (see ARCHITECTURE.md).
//!
//! The starting point is a byte scan that is *deliberately* not a parser: it
//! is exact enough for the handful of markup shapes search engines emit, and
//! it fails open (a mis-scan yields fewer hits, never a panic — every slice
//! is generated from `find`/`rfind` offsets).

/// Remove tags and collapse whitespace runs (titles wrap query terms in
/// `<b>`, so text arrives interleaved with inline markup).
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
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Decode the entities engines actually emit — the five XML ones, `nbsp`,
/// the typographic set, and numeric references (`&#39;`, `&#x27;`) which
/// appear constantly in markup-lifted titles.
pub(crate) fn unescape_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < s.len() {
        let rest = &s[i..];
        let ch = match rest.chars().next() {
            Some(c) => c,
            None => break,
        };
        if ch != '&' {
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        // An entity is short; anything longer is a literal ampersand.
        match rest[1..].find(';').filter(|n| *n <= 12) {
            Some(n) => match decode_entity(&rest[1..1 + n]) {
                Some(c) => {
                    out.push(c);
                    i += n + 2;
                }
                None => {
                    out.push('&');
                    i += 1;
                }
            },
            None => {
                out.push('&');
                i += 1;
            }
        }
    }
    out
}

fn decode_entity(name: &str) -> Option<char> {
    if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
        return u32::from_str_radix(hex, 16).ok().and_then(char::from_u32);
    }
    if let Some(dec) = name.strip_prefix('#') {
        return dec.parse::<u32>().ok().and_then(char::from_u32);
    }
    Some(match name {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        "nbsp" => ' ',
        // Spacing/zero-width entities the SERPs use as separators; mapping
        // them to a space keeps them from leaking into snippets verbatim.
        "ensp" | "emsp" | "thinsp" | "zwnj" | "zwj" => ' ',
        "hellip" => '…',
        "mdash" => '—',
        "ndash" => '–',
        "lsquo" => '‘',
        "rsquo" => '’',
        "ldquo" => '“',
        "rdquo" => '”',
        "middot" => '·',
        "times" => '×',
        "copy" => '©',
        "deg" => '°',
        "laquo" => '«',
        "raquo" => '»',
        "bull" => '•',
        "trade" => '™',
        "eacute" => 'é',
        "egrave" => 'è',
        "uuml" => 'ü',
        "ouml" => 'ö',
        "auml" => 'ä',
        "szlig" => 'ß',
        "ccedil" => 'ç',
        "ntilde" => 'ñ',
        _ => return None,
    })
}

/// Tag-free, entity-decoded, whitespace-collapsed text — what a title or
/// snippet fragment turns into before it reaches the model.
pub(crate) fn decode_text(s: &str) -> String {
    unescape_entities(&strip_tags(s)).trim().to_string()
}

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
    Some(Tag {
        open_tag,
        inner: hay[inner_start..end].to_string(),
    })
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

/// Byte offset of the opening `<` of the first element whose `class`
/// attribute lists `token` as a whole class (`class="result s-1"` matches
/// `result`; `class="result__a"` does not).
pub(crate) fn class_element(hay: &str, token: &str) -> Option<usize> {
    class_elements(hay, token).into_iter().next()
}

/// Byte offsets of every element whose class list contains `token`.
pub(crate) fn class_elements(hay: &str, token: &str) -> Vec<usize> {
    let mut found = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = hay[from..].find("class=") {
        let at = from + rel;
        let after = hay[at + "class=".len()..].trim_start();
        let quote = after.chars().next().unwrap_or('"');
        if (quote == '"' || quote == '\'')
            && let Some(end) = after[1..].find(quote)
        {
            let value = &after[1..1 + end];
            if value.split_whitespace().any(|c| c == token) {
                // Walk back to the tag's opening angle bracket.
                if let Some(open) = hay[..at].rfind('<') {
                    found.push(open);
                }
            }
            from = at + "class=".len() + 1 + end;
            continue;
        }
        from = at + "class=".len();
    }
    found
}

/// Slices of `hay`, one per element carrying `token`, each running to the
/// start of the next such element (the last one to the end of the input).
/// This is the "result row" splitter the engines' parsers walk.
pub(crate) fn class_blocks<'a>(hay: &'a str, token: &str) -> Vec<&'a str> {
    let starts = class_elements(hay, token);
    let mut blocks = Vec::with_capacity(starts.len());
    for (i, &start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(hay.len());
        blocks.push(&hay[start..end]);
    }
    blocks
}

/// Slices of `hay`, one per occurrence of `marker`, each running to the start
/// of the next occurrence (the last to the end). The generic row splitter for
/// markup that identifies rows by an attribute rather than a class.
pub(crate) fn marker_blocks<'a>(hay: &'a str, marker: &str) -> Vec<&'a str> {
    let mut starts = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = hay[from..].find(marker) {
        starts.push(from + rel);
        from += rel + marker.len();
    }
    let mut blocks = Vec::with_capacity(starts.len());
    for (i, &start) in starts.iter().enumerate() {
        let end = starts.get(i + 1).copied().unwrap_or(hay.len());
        blocks.push(&hay[start..end]);
    }
    blocks
}

/// Byte offset of the `<` opening the element that carries `marker` (the
/// reverse of [`marker_blocks`] for a single element).
pub(crate) fn marker_element(hay: &str, marker: &str) -> Option<usize> {
    let at = hay.find(marker)?;
    hay[..at].rfind('<')
}

/// The opening tag of the element starting at `offset` (which must point at
/// its `<`).
pub(crate) fn open_tag_at(hay: &str, offset: usize) -> Option<&str> {
    let gt = hay[offset..].find('>')? + offset;
    Some(&hay[offset..=gt])
}

/// The element's tag name, lowercased (`<A href=…>` → `"a"`).
fn tag_name(open_tag: &str) -> Option<String> {
    let rest = open_tag.strip_prefix('<')?;
    let end = rest.find(['>', ' ', '\t', '\n', '\r', '/'])?;
    if end == 0 {
        return None;
    }
    Some(rest[..end].to_ascii_lowercase())
}

/// Closers used when the matching one is out of the slice.
const CLOSERS: [&str; 9] = [
    "</div>", "</span>", "</p>", "</a>", "</li>", "</td>", "</h1>", "</h2>", "</h3>",
];

/// Inner HTML of the element whose `>` sits at `gt`, up to its **matching**
/// close tag — same-tag nesting counted, so a `div` holding a `span` does not
/// end at the span.
fn matching_inner(hay: &str, gt: usize, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut depth = 0usize;
    let mut i = gt + 1;
    while i < hay.len() {
        let next_open = hay[i..].find(&open).map(|n| i + n);
        let next_close = hay[i..].find(&close).map(|n| i + n)?;
        match next_open {
            Some(o) if o < next_close => {
                let boundary = hay[o + open.len()..].chars().next();
                if boundary.is_some_and(|c| matches!(c, '>' | ' ' | '\t' | '\n' | '\r' | '/')) {
                    depth += 1;
                }
                i = o + open.len();
            }
            _ => {
                if depth == 0 {
                    return Some(hay[gt + 1..next_close].to_string());
                }
                depth -= 1;
                i = next_close + close.len();
            }
        }
    }
    None
}

/// Text content of the element starting at `offset` (which must point at its
/// `<`).
pub(crate) fn text_at(hay: &str, offset: usize) -> Option<String> {
    let open = open_tag_at(hay, offset)?;
    let gt = offset + open.len() - 1; // the element's `>`
    let inner = match tag_name(open).and_then(|tag| matching_inner(hay, gt, &tag)) {
        Some(inner) => inner,
        // A windowed slice can end mid-element, leaving no matching closer;
        // the nearest closer still yields a usable partial snippet.
        None => {
            let rest = &hay[gt + 1..];
            let end = CLOSERS
                .iter()
                .filter_map(|closer| rest.find(closer))
                .min()?;
            rest[..end].to_string()
        }
    };
    Some(decode_text(&inner))
}

/// Text content of the first element whose class list contains `token`.
pub(crate) fn class_text(hay: &str, token: &str) -> Option<String> {
    let offset = class_element(hay, token)?;
    let text = text_at(hay, offset)?;
    (!text.is_empty()).then_some(text)
}

/// The opening tag of the nearest **unclosed** `tag` element before `pos` —
/// the string-scanning equivalent of `node.closest(tag)`. Engine result
/// anchors wrap their headings (`<a href=…><h3>Title</h3></a>`), so the href
/// has to be read from the ancestor, not from the heading.
pub(crate) fn enclosing_open_before(hay: &str, pos: usize, tag: &str) -> Option<String> {
    let needle = format!("<{tag}");
    let closer = format!("</{tag}>");
    let mut search_end = pos;
    while let Some(rel) = hay[..search_end].rfind(&needle) {
        let start = rel;
        // `<article` must not match `<a`.
        let boundary = hay[start + needle.len()..].chars().next();
        if boundary.is_some_and(|c| c.is_ascii_alphanumeric()) {
            search_end = start;
            continue;
        }
        let Some(gt) = hay[start..].find('>').map(|n| start + n) else {
            search_end = start;
            continue;
        };
        if gt >= pos {
            search_end = start;
            continue;
        }
        // Already closed before `pos`? Keep walking outwards.
        if hay[gt..pos].contains(&closer) {
            search_end = start;
            continue;
        }
        return Some(hay[start..=gt].to_string());
    }
    None
}

/// Inner HTML of the first `<form …>` whose opening tag contains `needle`.
pub(crate) fn form_with(hay: &str, needle: &str) -> Option<String> {
    let mut from = 0usize;
    while let Some(rel) = hay[from..].find("<form") {
        let start = from + rel;
        let gt = hay[start..].find('>')? + start;
        let open = &hay[start..=gt];
        let inner_end = hay[gt..]
            .find("</form>")
            .map(|n| gt + n)
            .unwrap_or(hay.len());
        if open.contains(needle) {
            return Some(hay[gt + 1..inner_end].to_string());
        }
        from = inner_end.max(gt + 1);
    }
    None
}

/// `(name, value)` of every `<input type="hidden">` inside a form body.
/// Startpage's search form must be replayed with its session token and
/// sibling hidden fields, so this reads them generically rather than
/// assuming an attribute order.
pub(crate) fn hidden_inputs(form: &str) -> Vec<(String, String)> {
    let mut inputs = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = form[from..].find("<input") {
        let start = from + rel;
        let Some(gt) = form[start..].find('>').map(|n| start + n) else {
            break;
        };
        let tag = &form[start..=gt];
        if (tag.contains("type=\"hidden\"") || tag.contains("type='hidden'"))
            && let (Some(name), Some(value)) =
                (extract_attr(tag, "name"), extract_attr(tag, "value"))
        {
            inputs.push((unescape_entities(&name), unescape_entities(&value)));
        }
        from = gt + 1;
    }
    inputs
}

/// Decoded text of every `<span>` in `hay`, in document order. Engines bury
/// publication timestamps in bare spans, so this is how a date is lifted out
/// of an otherwise uninteresting container.
pub(crate) fn spans(hay: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = hay[from..].find("<span") {
        let start = from + rel;
        let Some(gt) = hay[start..].find('>').map(|n| start + n) else {
            break;
        };
        let Some(end) = hay[gt..].find("</span>").map(|n| gt + n) else {
            break;
        };
        out.push(decode_text(&hay[gt + 1..end]));
        from = end + "</span>".len();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entities_decode_named_numeric_and_hex() {
        assert_eq!(unescape_entities("a &amp; b"), "a & b");
        assert_eq!(unescape_entities("&#39;quoted&#39;"), "'quoted'");
        assert_eq!(unescape_entities("&#x27;x&#x27;"), "'x'");
        assert_eq!(unescape_entities("caf&eacute;"), "café");
        // A bare ampersand, or one with an unknown name, survives verbatim.
        assert_eq!(unescape_entities("AT&T"), "AT&T");
        assert_eq!(unescape_entities("a &unknown; b"), "a &unknown; b");
        // Multi-byte neighbours must not be sliced apart.
        assert_eq!(unescape_entities("中文 &amp; 中文"), "中文 & 中文");
    }

    #[test]
    fn class_matching_is_whole_token_only() {
        let html = r#"<div class="result results_links">a</div><a class="result__a">b</a>"#;
        // `result__a` is not the class `result`, so only the div matches.
        assert_eq!(class_elements(html, "result").len(), 1);
        assert_eq!(class_elements(html, "result__a").len(), 1);
        // Attribute-order and quote-style independence.
        let html = r#"<div id=x class='result'>y</div>"#;
        assert_eq!(class_element(html, "result"), Some(0));
    }

    #[test]
    fn class_blocks_split_rows() {
        let html = r#"<div class="result">one</div><div class="result">two</div>"#;
        let blocks = class_blocks(html, "result");
        assert_eq!(blocks.len(), 2);
        assert!(blocks[0].contains("one"));
        assert!(blocks[1].contains("two"));
        assert_eq!(class_blocks("<p>none</p>", "result").len(), 0);
    }

    #[test]
    fn text_at_reads_a_row_without_a_parser() {
        let html = r#"<a class="result__snippet" href="x">Learn <b>async</b> Rust &amp; more</a>"#;
        let offset = class_element(html, "result__snippet").unwrap();
        assert_eq!(
            text_at(html, offset).as_deref(),
            Some("Learn async Rust & more")
        );
        assert_eq!(
            class_text(html, "result__snippet").as_deref(),
            Some("Learn async Rust & more")
        );
        assert_eq!(class_text(html, "absent"), None);
    }

    #[test]
    fn enclosing_open_before_finds_the_wrapping_anchor() {
        let html = r#"<div class="x"><a href="https://t.test/"><br><h3 class="LC20lb">Title</h3></a></div>"#;
        let h3 = html.find("<h3").unwrap();
        let tag = enclosing_open_before(html, h3, "a").unwrap();
        assert!(tag.contains("https://t.test/"), "{tag}");
        // `<article` must not be mistaken for `<a`; and a closed anchor is skipped.
        let html = r#"<article class="y">text</article>"#;
        assert_eq!(enclosing_open_before(html, html.len(), "a"), None);
        let html = r#"<a href="first">x</a><h3>T</h3>"#;
        assert_eq!(
            enclosing_open_before(html, html.find("<h3").unwrap(), "a"),
            None,
            "已闭合的锚不能当作祖先"
        );
    }

    #[test]
    fn hidden_inputs_are_read_regardless_of_attribute_order() {
        let form = r#"<form action="/sp/search"><input value="tok" type="hidden" name="sc"><input type="hidden" name="qid" value="42"><input type="text" name="query"></form>"#;
        let inputs = hidden_inputs(form);
        assert_eq!(
            inputs,
            vec![
                ("sc".to_string(), "tok".to_string()),
                ("qid".to_string(), "42".to_string())
            ]
        );
        // The non-hidden input is dropped (the caller supplies the query).
        assert!(!inputs.iter().any(|(n, _)| n == "query"));
    }

    #[test]
    fn form_with_matches_on_the_opening_tag() {
        let html = r#"<form action="/other"></form><form action="/sp/search"><input name="sc" type="hidden" value="v"></form>"#;
        let form = form_with(html, "action=\"/sp/search\"").unwrap();
        assert!(form.contains("name=\"sc\""), "{form}");
        assert!(!form.contains("</form>"), "内层不能含结束标签: {form}");
        assert_eq!(form_with(html, "action=\"/nope\""), None);
    }

    #[test]
    fn text_at_counts_nested_same_tag_elements() {
        // A container holding a span must not end at the span — that is
        // `textContent` semantics, and Ecosia's description container
        // (`<div …><span>caption</span>real text</div>`) depends on it.
        let html = r#"<div data-test-id="d"><span>caption</span>real text</div>"#;
        let offset = marker_element(html, "data-test-id=\"d\"").unwrap();
        assert_eq!(text_at(html, offset).as_deref(), Some("captionreal text"));

        let html = r#"<div class="a"><div class="b">inner</div>outer</div>"#;
        let offset = class_element(html, "a").unwrap();
        assert_eq!(text_at(html, offset).as_deref(), Some("innerouter"));

        // `<divx>` is not a nested `<div>`.
        let html = r#"<div class="a"><divx>nope</divx>yes</div>"#;
        let offset = class_element(html, "a").unwrap();
        assert_eq!(text_at(html, offset).as_deref(), Some("nopeyes"));

        // A slice that ends mid-element has no matching closer: the nearest
        // closer still yields a usable partial snippet.
        let truncated = r#"<div class="a">partial<p>more</p>"#;
        let offset = class_element(truncated, "a").unwrap();
        assert_eq!(text_at(truncated, offset).as_deref(), Some("partialmore"));
    }

    #[test]
    fn spans_are_listed_in_order() {
        let html = r#"<div class="result__extras__url"><span> page </span><span>&nbsp; &nbsp; 2026-07-30T20:19:00.0000000</span></div>"#;
        let texts = spans(html);
        assert_eq!(texts.len(), 2);
        assert_eq!(texts[1], "2026-07-30T20:19:00.0000000");
    }
}
