//! Baidu's results page, as a browser sees it.

use super::super::hit::Hit;
use super::super::provider::{SearchProvider, absolute};
use crate::web::html::{
    class_blocks, class_text, decode_text, enclosing_open_before, extract_attr, extract_between,
};
use crate::web::url::encode_query;

pub(super) const NAME: &str = "baidu";

pub(super) struct Baidu;

const HOME: &str = "https://www.baidu.com/";
const SEARCH_URL: &str = "https://www.baidu.com/s";
const SNIPPET_CLASSES: [&str; 4] = [
    "c-abstract",
    "cos-line-clamp-2",
    "cos-line-clamp-3",
    "cos-line-clamp-1",
];
/// The phrases a real, empty results page carries.
const EMPTY: [&str; 3] = ["抱歉没有找到", "没有找到与", "很抱歉，没有找到"];

/// Card containers carry the destination in an attribute; organic results only
/// have the engine's own redirect anchor.
fn block_href(block: &str) -> Option<String> {
    let open = &block[..block.find('>')? + 1];
    if let Some(direct) = extract_attr(open, "mu")
        && direct.starts_with("http")
    {
        return Some(direct);
    }
    let h3 = block.find("<h3")?;
    let inner = extract_between(&block[h3..], "<h3", "</h3>")?.inner;
    let href = extract_between(&inner, "<a", ">")
        .map(|tag| tag.open_tag)
        .or_else(|| enclosing_open_before(block, h3, "a"))
        .and_then(|tag| extract_attr(&tag, "href"))?;
    absolute(&href, HOME)
}

impl SearchProvider for Baidu {
    fn name(&self) -> &'static str {
        NAME
    }

    fn page_url(&self, query: &str, page: u32) -> String {
        format!(
            "{SEARCH_URL}?wd={}&pn={}&ie=utf-8",
            encode_query(query),
            10 * page.saturating_sub(1)
        )
    }

    fn parse(&self, html: &str) -> Vec<Hit> {
        let mut hits = Vec::new();
        for block in class_blocks(html, "c-container") {
            let Some(h3) = extract_between(block, "<h3", "</h3>") else {
                continue;
            };
            let title = decode_text(&h3.inner);
            if title.is_empty() {
                continue;
            }
            let Some(url) = block_href(block) else {
                continue;
            };
            let snippet = SNIPPET_CLASSES
                .iter()
                .find_map(|token| class_text(block, token))
                .unwrap_or_default();
            hits.push(Hit::new(title, url, snippet));
        }
        hits
    }

    fn normal(&self, html: &str) -> bool {
        !self.parse(html).is_empty() || EMPTY.iter().any(|marker| html.contains(marker))
    }
}
