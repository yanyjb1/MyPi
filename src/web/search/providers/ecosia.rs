//! Ecosia — HTML scrape of the server-rendered (Vue/Nuxt) results page,
//! ported from oh-my-pi's `ecosia.ts`.
//!
//! Ecosia serves Google-backed results and fronts search with Cloudflare, so
//! the plain fetch usually works and the render only happens when the firewall
//! answers instead.
//!
//! `recency` is deliberately ignored: Ecosia's web results expose no date
//! filter in its UI and the legacy `freshness` parameter is a server-side
//! no-op (omp verified this live), so the field must not be approximated by
//! rewriting the query.

use super::super::hit::SearchHit;
use super::super::http::encode;
use super::super::query::{GOOGLE_QUERY_SYNTAX, format_scraper_query};
use super::{BrowserPlan, Ctx, absolute, finish, get, is_external};
use crate::web::utils::html::{
    enclosing_open_before, extract_attr, marker_blocks, marker_element, text_at,
};

pub const LABEL: &str = "Ecosia";
const HOME: &str = "https://www.ecosia.org/";
const SEARCH_URL: &str = "https://www.ecosia.org/search";

pub(super) fn build_url(ctx: &Ctx<'_>) -> String {
    let query = format_scraper_query(ctx.query, Some(ctx.parsed), GOOGLE_QUERY_SYNTAX);
    format!("{SEARCH_URL}?q={}", encode(&query))
}

/// Ecosia's Cloudflare front answers with a 403 titled "Ecosia Firewall"
/// carrying the `_cf_chl_opt` bootstrap and the challenge-platform loader.
pub(super) fn is_blocked(status: u16, _url: &str, body: &str) -> Option<&'static str> {
    if status == 403
        || status == 429
        || body.contains("Ecosia Firewall")
        || body.contains("_cf_chl_opt")
        || body.contains("/cdn-cgi/challenge-platform/")
        || body.to_lowercase().contains("not a robot")
    {
        return Some("blocked the request with a Cloudflare bot challenge");
    }
    None
}

/// Text of the first element carrying `marker`, given the block slice.
fn text_of_marker(block: &str, marker: &str) -> Option<String> {
    let offset = marker_element(block, marker)?;
    let text = text_at(block, offset)?;
    (!text.is_empty()).then_some(text)
}

/// Organic results in document order. Each is an
/// `<article data-test-id="organic-result">` whose title heading is wrapped by
/// the target anchor; ad slots and entity cards use different test ids, so
/// they never match.
pub(super) fn parse_html(html: &str) -> Vec<SearchHit> {
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
        // The description paragraph is preferred over its container: the
        // container also holds screen-reader-only thumbnail captions.
        let snippet = text_of_marker(block, "data-test-id=\"web-result-description\"")
            .or_else(|| text_of_marker(block, "data-test-id=\"result-description\""))
            .unwrap_or_default();
        hits.push(SearchHit::new(LABEL, title, url, snippet));
    }
    hits
}

pub fn search(ctx: &Ctx<'_>) -> anyhow::Result<Vec<SearchHit>> {
    let url = build_url(ctx);
    let direct = get(&url, Some(HOME));
    let plan = BrowserPlan {
        url: url.clone(),
        warmup: Some(HOME),
        prelude: None,
    };
    finish(ctx, LABEL, direct, plan, parse_html, is_blocked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::ai::config::BrowserConfig;

    const FIXTURE: &str = r#"
<div data-test-id="ad-google"><a href="https://ad.test/"><h2 data-test-id="result-title">Ad</h2></a></div>
<article data-test-id="organic-result">
  <a class="result__link" href="https://www.rust-lang.org/"><h2 data-test-id="result-title" class="result__title">Rust Programming Language</h2></a>
  <p data-test-id="web-result-description">A language empowering everyone.</p>
</article>
<article data-test-id="organic-result">
  <a class="result__link" href="https://ecosia.org/images?q=rust"><h2 data-test-id="result-title">Images</h2></a>
  <p data-test-id="web-result-description">internal</p>
</article>
<article data-test-id="organic-result">
  <a class="result__link" href="https://tokio.rs/"><h2 data-test-id="result-title">Tokio</h2></a>
  <div data-test-id="result-description"><span>caption</span>An async runtime.</div>
</article>
"#;

    #[test]
    fn parses_organic_results_and_skips_ads_and_internal_links() {
        let hits = parse_html(FIXTURE);
        assert_eq!(hits.len(), 2, "{hits:#?}");
        assert_eq!(hits[0].title, "Rust Programming Language");
        assert_eq!(hits[0].url, "https://www.rust-lang.org/");
        assert_eq!(hits[0].snippet, "A language empowering everyone.");
        // The description container is the fallback when the paragraph is absent.
        assert_eq!(hits[1].url, "https://tokio.rs/");
        assert!(
            hits[1].snippet.contains("An async runtime."),
            "{}",
            hits[1].snippet
        );
    }

    #[test]
    fn cloudflare_and_firewall_pages_are_walls() {
        assert!(is_blocked(403, SEARCH_URL, "").is_some());
        assert!(is_blocked(200, SEARCH_URL, "<title>Ecosia Firewall</title>").is_some());
        assert!(is_blocked(200, SEARCH_URL, "var _cf_chl_opt={}").is_some());
        assert!(is_blocked(200, SEARCH_URL, "Please confirm you're not a robot").is_some());
        assert!(is_blocked(200, SEARCH_URL, "<article data-test-id=\"organic-result\">").is_none());
    }

    #[test]
    fn recency_is_not_approximated_in_the_query() {
        let browser = BrowserConfig::default();
        let parsed = crate::web::search::query::parse("rust");
        let ctx = Ctx {
            query: "rust",
            parsed: &parsed,
            limit: 10,
            recency: Some(super::super::Recency::Day),
            browser: &browser,
        };
        let url = build_url(&ctx);
        assert_eq!(
            url,
            format!("{SEARCH_URL}?q=rust"),
            "Ecosia 没有可用的时间过滤"
        );
    }
}
