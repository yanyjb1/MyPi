use super::super::hit::Hit;
use super::super::provider::{SearchProvider, absolute, is_external};
use crate::web::html::{
    class_element, class_text, extract_attr, extract_between, marker_blocks, open_tag_at, text_at,
};
use crate::web::url::encode_query;

pub(super) struct Mojeek;

const HOME: &str = "https://www.mojeek.de/?arc=none&lang=en&lb=en&theme=dark";
const SEARCH_URL: &str = "https://www.mojeek.de/search";
const SELF_HOSTS: [&str; 4] = ["mojeek.com", "mojeek.co.uk", "mojeek.fr", "mojeek.de"];

impl SearchProvider for Mojeek {
    fn name(&self) -> &'static str {
        "mojeek"
    }

    fn page_url(&self, query: &str, page: u32) -> String {
        format!(
            "{SEARCH_URL}?q={}&s={}&arc=none&lang=en&lb=en&theme=dark",
            encode_query(query),
            10 * page.saturating_sub(1)
        )
    }

    fn parse(&self, html: &str) -> Vec<Hit> {
        let mut hits = Vec::new();
        let Some(list) = extract_between(html, "<ul class=\"results-standard\"", "</ul>") else {
            return hits;
        };
        for block in marker_blocks(&list.inner, "<li") {
            let Some(anchor) = class_element(block, "title") else {
                continue;
            };
            let Some(href) = open_tag_at(block, anchor).and_then(|tag| extract_attr(tag, "href"))
            else {
                continue;
            };
            let Some(url) = absolute(&href, HOME) else {
                continue;
            };
            if !is_external(&url, &SELF_HOSTS) {
                continue;
            }
            let title = text_at(block, anchor).unwrap_or_default();
            if title.is_empty() {
                continue;
            }
            hits.push(Hit::new(
                title,
                url,
                class_text(block, "s").unwrap_or_default(),
            ));
        }
        hits
    }

    fn challenge(&self, _url: &str, html: &str) -> Option<&'static str> {
        let wall = html.contains("altcha-widget")
            || html.contains("captcha-wrap")
            || html.to_lowercase().contains("sending automated queries");
        (wall && !html.contains("results-standard"))
            .then_some("blocked the request with its automated-queries wall")
    }
}
