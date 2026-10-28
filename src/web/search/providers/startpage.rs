use super::super::hit::Hit;
use super::super::provider::{SearchProvider, absolute, is_external};
use crate::web::html::{
    class_blocks, class_element, class_text, extract_attr, open_tag_at, text_at,
};
use crate::web::url::encode_query;

pub(super) struct Startpage;

const HOME: &str = "https://www.startpage.com/";
const SEARCH_URL: &str = "https://www.startpage.com/sp/search";

impl SearchProvider for Startpage {
    fn name(&self) -> &'static str {
        "startpage"
    }

    fn page_url(&self, query: &str, page: u32) -> String {
        format!("{SEARCH_URL}?query={}&page={page}", encode_query(query))
    }

    fn parse(&self, html: &str) -> Vec<Hit> {
        let mut hits = Vec::new();
        for block in class_blocks(html, "result") {
            let Some(anchor) = class_element(block, "result-link") else {
                continue;
            };
            let Some(href) = open_tag_at(block, anchor).and_then(|tag| extract_attr(tag, "href"))
            else {
                continue;
            };
            let Some(url) = absolute(&href, HOME) else {
                continue;
            };
            if !is_external(&url, &["startpage.com"]) {
                continue;
            }
            let title = text_at(block, anchor).unwrap_or_default();
            if title.is_empty() {
                continue;
            }
            hits.push(Hit::new(
                title,
                url,
                class_text(block, "description").unwrap_or_default(),
            ));
        }
        hits
    }

    fn challenge(&self, url: &str, html: &str) -> Option<&'static str> {
        if html.contains("anubis_challenge") || html.contains("anubis_version") {
            return Some("blocked the request with a proof-of-work challenge (Anubis)");
        }
        let challenged = url.contains("/errors/")
            || url.contains("/captcha/")
            || url.contains("/sp/captcha")
            || html.contains("component---src-pages-captcha")
            || html.contains("/sp/captcha");
        challenged.then_some("blocked the request with a CAPTCHA challenge")
    }
}
