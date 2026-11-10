//! Bing's results page, as a browser sees it.

use super::super::hit::Hit;
use super::super::provider::SearchProvider;
use crate::web::html::{
    class_blocks, class_text, decode_text, extract_attr, extract_between, strip_tags,
    unescape_entities,
};
use crate::web::url::{encode_query, percent_decode};

pub(super) const NAME: &str = "bing";

pub(super) struct Bing;

const SEARCH_URL: &str = "https://www.bing.com/search";
/// What an empty Bing results page says.
const EMPTY: [&str; 1] = ["There are no results for"];

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
        NAME
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
        for block in class_blocks(html, "b_algo") {
            let Some(h2) = extract_between(block, "<h2", "</h2>") else {
                continue;
            };
            let Some(a_open) = extract_between(&h2.inner, "<a", ">").map(|t| t.open_tag) else {
                continue;
            };
            let Some(href) = extract_attr(&a_open, "href") else {
                continue;
            };
            let title = decode_text(&h2.inner);
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
                let p = extract_between(&block[pos..], "<p", "</p>")?;
                let text = unescape_entities(strip_tags(&p.inner).trim());
                (!text.is_empty()).then_some(text)
            })
            .or_else(|| class_text(block, "b_caption"))
            .unwrap_or_default();
            hits.push(Hit::new(title, url, snippet));
        }
        hits
    }

    fn normal(&self, html: &str) -> bool {
        !self.parse(html).is_empty() || EMPTY.iter().any(|marker| html.contains(marker))
    }
}
