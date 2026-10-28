use super::super::hit::Hit;
use super::super::provider::{SearchProvider, absolute, is_external};
use crate::web::html::{
    class_blocks, class_element, extract_attr, open_tag_at, text_at, unescape_entities,
};
use crate::web::url::{encode_query, percent_decode};

pub(super) struct DuckDuckGo;

const HOME: &str = "https://html.duckduckgo.com/";
const SERP_URL: &str = "https://html.duckduckgo.com/html/";

fn unwrap_result_url(href: &str) -> Option<String> {
    let decoded = unescape_entities(href);
    if let Some(pos) = decoded.find("uddg=") {
        let rest = &decoded[pos + "uddg=".len()..];
        let end = rest.find('&').unwrap_or(rest.len());
        let target = percent_decode(&rest[..end]);
        if target.starts_with("http") {
            return Some(target);
        }
    }
    let url = absolute(decoded.as_str(), HOME)?;
    is_external(&url, &["duckduckgo.com"]).then_some(url)
}

impl SearchProvider for DuckDuckGo {
    fn name(&self) -> &'static str {
        "duckduckgo"
    }

    fn page_url(&self, query: &str, page: u32) -> String {
        format!(
            "{SERP_URL}?q={}&s={}",
            encode_query(query),
            10 * page.saturating_sub(1)
        )
    }

    fn parse(&self, html: &str) -> Vec<Hit> {
        let mut hits = Vec::new();
        for block in class_blocks(html, "result") {
            let Some(anchor) = class_element(block, "result__a") else {
                continue;
            };
            let Some(href) = open_tag_at(block, anchor).and_then(|tag| extract_attr(tag, "href"))
            else {
                continue;
            };
            let Some(url) = unwrap_result_url(&href) else {
                continue;
            };
            let title = text_at(block, anchor).unwrap_or_default();
            if title.is_empty() {
                continue;
            }
            let snippet = class_element(block, "result__snippet")
                .and_then(|offset| text_at(block, offset))
                .unwrap_or_default();
            hits.push(Hit::new(title, url, snippet));
        }
        hits
    }

    fn challenge(&self, _url: &str, html: &str) -> Option<&'static str> {
        (html.contains("anomaly-modal") || html.contains("anomaly.js"))
            .then_some("blocked the request with a bot-detection challenge")
    }
}
