use super::super::hit::Hit;
use super::super::provider::{SearchProvider, absolute, is_external};
use crate::web::html::{
    class_text, decode_text, enclosing_open_before, extract_attr, extract_between,
};
use crate::web::url::encode_query;

pub(super) struct Google;

const HOME: &str = "https://www.google.com/";
const SEARCH_URL: &str = "https://www.google.com/search";
const SNIPPET_CLASSES: [&str; 4] = ["VwiC3b", "IsZvec", "s3v9rd", "BNeawe"];

fn has_traffic_warning(body: &str) -> bool {
    let lower = body.to_lowercase();
    lower.contains("unusual traffic") || lower.contains("g-recaptcha")
}

fn unwrap_result_url(href: &str) -> Option<String> {
    let direct = absolute(href, HOME)?;
    let mut url = url::Url::parse(&direct).ok()?;
    let host = url.host_str().unwrap_or_default().to_lowercase();
    if (host == "google.com" || host == "www.google.com") && url.path() == "/url" {
        let target = url
            .query_pairs()
            .find(|(k, _)| k == "q" || k == "url")
            .map(|(_, v)| v.to_string())?;
        url = url::Url::parse(&target).ok()?;
    }
    if !is_external(url.as_str(), &["google.com"]) {
        return None;
    }
    Some(url.to_string())
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
        "google"
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
            let Some(url) = unwrap_result_url(&href) else {
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

    fn challenge(&self, url: &str, html: &str) -> Option<&'static str> {
        if html.contains("/httpservice/retry/enablejs") && !html.contains("<h3") {
            return Some("returned its JavaScript challenge instead of results");
        }
        if url.contains("/sorry/") || has_traffic_warning(html) {
            return Some("blocked the request with an automated-traffic challenge");
        }
        None
    }
}
