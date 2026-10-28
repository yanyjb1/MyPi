use super::super::hit::Hit;
use super::super::provider::{SearchProvider, absolute, is_external};
use crate::web::html::{
    enclosing_open_before, extract_attr, marker_blocks, marker_element, text_at,
};
use crate::web::url::encode_query;

pub(super) struct Ecosia;

const HOME: &str = "https://www.ecosia.org/";
const SEARCH_URL: &str = "https://www.ecosia.org/search";

fn text_of_marker(block: &str, marker: &str) -> Option<String> {
    let offset = marker_element(block, marker)?;
    let text = text_at(block, offset)?;
    (!text.is_empty()).then_some(text)
}

impl SearchProvider for Ecosia {
    fn name(&self) -> &'static str {
        "ecosia"
    }

    fn page_url(&self, query: &str, page: u32) -> String {
        format!("{SEARCH_URL}?q={}&p={page}", encode_query(query))
    }

    fn parse(&self, html: &str) -> Vec<Hit> {
        let mut hits = Vec::new();
        for block in marker_blocks(html, "data-test-id=\"organic-result\"") {
            let Some(offset) = marker_element(block, "data-test-id=\"result-title\"") else {
                continue;
            };
            let Some(anchor) = enclosing_open_before(block, offset, "a") else {
                continue;
            };
            let Some(href) = extract_attr(&anchor, "href") else {
                continue;
            };
            let Some(url) = absolute(&href, HOME) else {
                continue;
            };
            if !is_external(&url, &["ecosia.org"]) {
                continue;
            }
            let Some(title) = text_at(block, offset) else {
                continue;
            };
            if title.is_empty() {
                continue;
            }
            let snippet = text_of_marker(block, "data-test-id=\"web-result-description\"")
                .or_else(|| text_of_marker(block, "data-test-id=\"result-description\""))
                .unwrap_or_default();
            hits.push(Hit::new(title, url, snippet));
        }
        hits
    }

    fn challenge(&self, _url: &str, html: &str) -> Option<&'static str> {
        if html.contains("Ecosia Firewall")
            || html.contains("_cf_chl_opt")
            || html.contains("/cdn-cgi/challenge-platform/")
            || html.to_lowercase().contains("not a robot")
        {
            return Some("blocked the request with a Cloudflare bot challenge");
        }
        None
    }
}
