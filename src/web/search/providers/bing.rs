use super::super::hit::Hit;
use super::super::provider::SearchProvider;
use crate::web::html::{extract_attr, extract_between, strip_tags, unescape_entities};
use crate::web::url::{encode_query, percent_decode};

pub(super) struct Bing;

const SEARCH_URL: &str = "https://www.bing.com/search";

fn unwrap_tracking_href(href: &str) -> String {
    let href = unescape_entities(href);
    if let Some(pos) = href.find("&u=a1") {
        let enc = &href[pos + 5..];
        let end = enc.find('&').unwrap_or(enc.len());
        let enc = &enc[..end];
        if let Some(bytes) = base64url_decode(enc)
            && let Ok(s) = String::from_utf8(bytes)
            && s.starts_with("http")
        {
            return s;
        }
        return href;
    }
    if let Some(pos) = href.find("&u=http") {
        let rest = &href[pos + 3..];
        let end = rest.find('&').unwrap_or(rest.len());
        return percent_decode(&rest[..end]);
    }
    href
}

fn base64url_decode(s: &str) -> Option<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a') as u32 + 26),
            b'0'..=b'9' => Some((c - b'0') as u32 + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for &c in bytes {
        let v = val(c)?;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

impl SearchProvider for Bing {
    fn name(&self) -> &'static str {
        "bing"
    }

    fn page_url(&self, query: &str, page: u32) -> String {
        format!(
            "{SEARCH_URL}?q={}&count=10&first={}",
            encode_query(query),
            10 * page.saturating_sub(1) + 1
        )
    }

    fn parse(&self, html: &str) -> Vec<Hit> {
        let mut hits = Vec::new();
        let pat = "<li class=\"b_algo";
        let mut from = 0usize;
        while let Some(start) = html[from..].find(pat) {
            let s = from + start;
            let next = html[s + pat.len()..]
                .find(pat)
                .map(|n| s + pat.len() + n)
                .unwrap_or(html.len());
            let block = &html[s..next];
            from = next;

            let Some(h2) = extract_between(block, "<h2", "</h2>") else {
                continue;
            };
            let Some(a_open) = extract_between(&h2.inner, "<a", ">").map(|t| t.open_tag) else {
                continue;
            };
            let Some(href) = extract_attr(&a_open, "href") else {
                continue;
            };
            let title = strip_tags(&h2.inner);
            if title.is_empty() {
                continue;
            }
            let url = unwrap_tracking_href(&href);
            let snippet = [
                "<p class=\"b_lineclamp",
                "<p class=\"b_algoSlug",
                "<p class=\"b_caption",
            ]
            .iter()
            .find_map(|marker| {
                let pos = block.find(marker)?;
                let seg = &block[pos..];
                let p = extract_between(seg, "<p", "</p>")?;
                let text = unescape_entities(strip_tags(&p.inner).trim());
                (!text.is_empty()).then_some(text)
            })
            .unwrap_or_default();
            hits.push(Hit::new(title, url, snippet));
        }
        hits
    }

    fn challenge(&self, url: &str, html: &str) -> Option<&'static str> {
        let lower = html.to_lowercase();
        if url.contains("/ck/")
            || lower.contains("captcha")
            || lower.contains("unusual traffic")
            || lower.contains("verify you are human")
        {
            return Some("blocked the request with a bot challenge");
        }
        None
    }
}
