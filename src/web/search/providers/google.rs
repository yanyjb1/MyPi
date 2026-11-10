//! Google's results page, as a browser sees it.

use super::super::hit::Hit;
use super::super::provider::{SearchProvider, absolute, is_external};
use crate::web::html::{
    class_text, decode_text, enclosing_open_before, extract_attr, extract_between,
};
use crate::web::url::encode_query;

pub(super) const NAME: &str = "google";

pub(super) struct Google;

const HOME: &str = "https://www.google.com/";
const SEARCH_URL: &str = "https://www.google.com/search";
const SNIPPET_CLASSES: [&str; 4] = ["VwiC3b", "IsZvec", "s3v9rd", "BNeawe"];
/// The two phrases a real, empty results page carries.
const EMPTY: [&str; 2] = ["did not match any documents", "No results found for"];

/// An anchor's href, when it can lead to a result: the engine's redirect shape
/// or an external page. Nav links (`/search?…`, `/preferences`) are dropped here,
/// so a gate page full of policy links parses to nothing.
fn result_href(href: &str) -> Option<String> {
    let direct = absolute(href, HOME)?;
    let parsed = url::Url::parse(&direct).ok()?;
    let host = parsed.host_str().unwrap_or_default().to_lowercase();
    let on_google = host == "google.com" || host == "www.google.com";
    if on_google && matches!(parsed.path(), "/goto" | "/url") {
        let carries = parsed
            .query_pairs()
            .any(|(key, _)| key == "url" || key == "q");
        return carries.then_some(direct);
    }
    is_external(&direct, &["google.com"]).then_some(direct)
}

fn find_snippet(html: &str, h3: usize) -> Option<String> {
    let window_end = html[h3 + 3..]
        .find("<h3")
        .map(|n| h3 + 3 + n)
        .unwrap_or(html.len());
    let window = &html[h3..window_end];
    SNIPPET_CLASSES
        .iter()
        .find_map(|token| class_text(window, token))
}

impl SearchProvider for Google {
    fn name(&self) -> &'static str {
        NAME
    }

    fn page_url(&self, query: &str, page: u32) -> String {
        format!(
            "{SEARCH_URL}?q={}&num=10&hl=en&gl=us&udm=14&pws=0&start={}",
            encode_query(query),
            10 * page.saturating_sub(1)
        )
    }

    fn parse(&self, html: &str) -> Vec<Hit> {
        let mut hits = Vec::new();
        let mut from = 0usize;
        while let Some(rel) = html[from..].find("<h3") {
            let h3 = from + rel;
            from = h3 + 3;
            let Some(anchor) = enclosing_open_before(html, h3, "a") else {
                continue;
            };
            let Some(href) = extract_attr(&anchor, "href") else {
                continue;
            };
            let Some(url) = result_href(&href) else {
                continue;
            };
            let title = extract_between(&html[h3..], "<h3", "</h3>")
                .map(|tag| decode_text(&tag.inner))
                .unwrap_or_default();
            if title.is_empty() {
                continue;
            }
            hits.push(Hit::new(
                title,
                url,
                find_snippet(html, h3).unwrap_or_default(),
            ));
        }
        hits
    }

    fn normal(&self, html: &str) -> bool {
        !self.parse(html).is_empty() || EMPTY.iter().any(|marker| html.contains(marker))
    }
}
