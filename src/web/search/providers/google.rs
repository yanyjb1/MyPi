//! Google — HTML scrape of the server-rendered results page, ported from
//! oh-my-pi's `google.ts`.
//!
//! Fetch-first with a headless-browser escalation: the plain GET with a
//! browser identity usually returns the whole SERP, and only a JS challenge or
//! an automated-traffic wall forces a render. `udm=14` asks for the web-only
//! vertical (no AI overview, no shopping carousel), which is also the markup
//! this parser knows.
//!
//! No API key, and no API: Google's programmatic search is not available
//! without one, so this engine is best-effort by construction.

use super::super::hit::SearchHit;
use super::super::http::encode;
use super::super::query::{GOOGLE_QUERY_SYNTAX, format_scraper_query};
use super::{BrowserPlan, Ctx, absolute, finish, get, is_external};
use crate::web::utils::html::{
    class_text, decode_text, enclosing_open_before, extract_attr, extract_between,
};

pub const LABEL: &str = "Google";
const HOME: &str = "https://www.google.com/";
const SEARCH_URL: &str = "https://www.google.com/search";

/// Snippet containers, most specific first. Google A/B-serves several class
/// names for the same paragraph, and this order is the parser's whole
/// knowledge of which one is authoritative.
const SNIPPET_CLASSES: [&str; 4] = ["VwiC3b", "IsZvec", "s3v9rd", "BNeawe"];

pub(super) fn build_url(ctx: &Ctx<'_>) -> String {
    let query = format_scraper_query(ctx.query, Some(ctx.parsed), GOOGLE_QUERY_SYNTAX);
    let mut url = format!(
        "{SEARCH_URL}?q={}&num={}&hl=en&gl=us&udm=14&pws=0",
        encode(&query),
        ctx.limit
    );
    if let Some(recency) = ctx.recency {
        url.push_str("&tbs=qdr:");
        url.push_str(recency.letter());
    }
    url
}

/// `true` when the body carries Google's "unusual traffic" interstitial.
fn has_traffic_warning(body: &str) -> bool {
    let lower = body.to_lowercase();
    lower.contains("unusual traffic") || lower.contains("g-recaptcha")
}

pub(super) fn is_blocked(status: u16, url: &str, body: &str) -> Option<&'static str> {
    if body.contains("/httpservice/retry/enablejs") && !body.contains("<h3") {
        return Some("returned its JavaScript challenge instead of results");
    }
    if status == 403 || status == 429 || url.contains("/sorry/") || has_traffic_warning(body) {
        return Some("blocked the request with an automated-traffic challenge");
    }
    None
}

/// Unwrap Google's `/url?q=<target>` click wrapper and drop self-links. The
/// target URL is also the only reliable "is this an organic result" signal:
/// the SERP's own navigation rows point back at google.com.
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

/// Snippet for the result whose heading starts at `h3`: the first snippet
/// container after the heading and before the next one.
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

/// Every organic result, in document order. The heading is the anchor: the
/// href lives on the enclosing `<a>`, the title is the heading's own text.
pub(super) fn parse_html(html: &str) -> Vec<SearchHit> {
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
        hits.push(SearchHit::new(
            LABEL,
            title,
            url,
            find_snippet(html, h3).unwrap_or_default(),
        ));
    }
    hits
}

pub fn search(ctx: &Ctx<'_>) -> anyhow::Result<Vec<SearchHit>> {
    let url = build_url(ctx);
    let direct = get(&url, None);
    let plan = BrowserPlan {
        url: url.clone(),
        warmup: Some(HOME),
        prelude: None,
    };
    finish(ctx, LABEL, direct, plan, parse_html, is_blocked)
}

#[cfg(test)]
mod tests {
    use super::super::Recency;
    use super::*;
    use crate::server::ai::config::BrowserConfig;

    /// Trimmed from a `udm=14` SERP: the anchor wraps the heading, the
    /// snippet sits in a sibling container.
    const FIXTURE: &str = r#"
<div class="MjjYud">
  <div class="tF2Cxc">
    <div class="yuRUbf"><a href="/url?q=https://doc.rust-lang.org/book/&amp;sa=U&amp;ved=2ah"><br><h3 class="LC20lb MBeuO DKV0Md">The Rust Programming Language</h3></a></div>
    <div class="VwiC3b yXK7lf">The official book. It is <b>free</b> &amp; online.</div>
  </div>
</div>
<div class="MjjYud">
  <div class="tF2Cxc">
    <div class="yuRUbf"><a href="https://tokio.rs/"><h3 class="LC20lb">Tokio</h3></a></div>
    <div class="IsZvec">An asynchronous runtime.</div>
  </div>
</div>
<a href="/search?q=more&amp;udm=14"><h3>Next page</h3></a>
"#;

    #[test]
    fn parses_titles_urls_and_snippets() {
        let hits = parse_html(FIXTURE);
        assert_eq!(hits.len(), 2, "自链接（/search）不该成为结果: {hits:#?}");
        assert_eq!(hits[0].title, "The Rust Programming Language");
        assert_eq!(hits[0].url, "https://doc.rust-lang.org/book/");
        assert_eq!(hits[0].snippet, "The official book. It is free & online.");
        assert_eq!(hits[0].provider, "Google");
        assert_eq!(hits[1].url, "https://tokio.rs/");
        assert_eq!(hits[1].snippet, "An asynchronous runtime.");
    }

    #[test]
    fn wrapper_unwrapping_rejects_self_links_and_junk() {
        assert_eq!(
            unwrap_result_url("/url?q=https%3A%2F%2Fa.test%2Fp&sa=U").as_deref(),
            Some("https://a.test/p")
        );
        assert_eq!(unwrap_result_url("/search?q=x"), None, "自链接被拒");
        assert_eq!(unwrap_result_url("javascript:void(0)"), None);
        assert_eq!(unwrap_result_url("about:blank"), None);
    }

    #[test]
    fn url_carries_the_web_vertical_and_the_recency_window() {
        let browser = BrowserConfig::default();
        let parsed = crate::web::search::query::parse("rust");
        let ctx = Ctx {
            query: "rust",
            parsed: &parsed,
            limit: 10,
            recency: Some(Recency::Week),
            browser: &browser,
        };
        let url = build_url(&ctx);
        assert!(url.contains("udm=14"), "{url}");
        assert!(url.contains("num=10"), "{url}");
        assert!(url.contains("tbs=qdr:w"), "{url}");

        let ctx = Ctx {
            recency: None,
            ..ctx
        };
        assert!(
            !build_url(&ctx).contains("tbs="),
            "无 recency 时不带时间过滤"
        );
    }

    #[test]
    fn walls_are_recognized_by_body_and_status() {
        assert!(is_blocked(200, SEARCH_URL, "/httpservice/retry/enablejs?sei=x").is_some());
        assert!(is_blocked(200, SEARCH_URL, "<h3>ok</h3>").is_none());
        assert!(is_blocked(403, SEARCH_URL, "").is_some());
        assert!(is_blocked(200, "https://www.google.com/sorry/index", "").is_some());
        assert!(
            is_blocked(
                200,
                SEARCH_URL,
                "We detected unusual traffic from your network"
            )
            .is_some()
        );
        // A JS challenge that DID render results is not a wall.
        assert!(is_blocked(200, SEARCH_URL, "/httpservice/retry/enablejs <h3>T</h3>").is_none());
    }

    #[test]
    fn empty_or_headingless_pages_yield_nothing() {
        assert!(parse_html("").is_empty());
        assert!(parse_html("<div>consent</div>").is_empty());
    }
}
