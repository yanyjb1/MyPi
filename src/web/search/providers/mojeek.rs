//! Mojeek — HTML scrape of its own index, ported from oh-my-pi's
//! `mojeek.ts`.
//!
//! Mojeek is the one keyless engine here with a genuinely independent index
//! rather than a Google/Bing reseller, which is why it stays in the chain
//! last: for the queries the other engines answer, it adds nothing, and for
//! the ones they refuse, it is the only answer.
//!
//! It also fronts results with an ALTCHA proof-of-work interstitial. The
//! browser tier clicks the widget's checkbox once before polling — the same
//! move omp makes — and if the challenge still stands the engine reports a
//! wall.

use std::time::Duration;

use super::super::hit::SearchHit;
use super::super::http::encode;
use super::super::query::{QuerySyntax, format_query};
use super::{BrowserPlan, Ctx, absolute, finish, get, is_external};
use crate::web::utils::html::{
    class_element, class_text, extract_attr, extract_between, marker_blocks, open_tag_at, text_at,
};
use crate::web::utils::session::Page;

pub const LABEL: &str = "Mojeek";
const HOME: &str = "https://www.mojeek.de/?arc=none&lang=en&lb=en&theme=dark";
const SEARCH_URL: &str = "https://www.mojeek.de/search";

/// Mojeek parses `site:`, quoted phrases and `-` exclusions. Its `in*:`
/// operators and its own `since:`/`before:` date syntax (YYYYMMDD) differ from
/// Google's ISO form, and `since` is already claimed by `recency`, so those
/// constraints are left to the lenient post-filter instead of being
/// mis-emitted.
const QUERY_SYNTAX: QuerySyntax = QuerySyntax {
    phrases: true,
    negation: true,
    or: false,
    site: true,
    in_url: false,
    in_title: false,
    in_text: false,
    filetype: false,
    date_range: false,
};

/// Domains that mean "a navigation row on the SERP", not a result.
const SELF_HOSTS: [&str; 4] = ["mojeek.com", "mojeek.co.uk", "mojeek.fr", "mojeek.de"];

pub(super) fn build_url(ctx: &Ctx<'_>) -> String {
    let query = format_query(ctx.parsed, QUERY_SYNTAX);
    let mut url = format!(
        "{SEARCH_URL}?q={}&t={}&arc=none&lang=en&lb=en&theme=dark",
        encode(&query),
        ctx.limit
    );
    // `since` takes the relative token verbatim (verified live: each window
    // returns a near-disjoint, fresher result set).
    if let Some(recency) = ctx.recency {
        url.push_str("&since=");
        url.push_str(recency.word());
    }
    url
}

/// The ALTCHA interstitial arrives as HTTP 200 (`<title>Captcha</title>`
/// carrying `altcha-widget`), and the "automated queries" refusal as 403.
/// Both bodies are more actionable than their statuses.
pub(super) fn is_blocked(_status: u16, _url: &str, body: &str) -> Option<&'static str> {
    let wall = body.contains("altcha-widget")
        || body.contains("captcha-wrap")
        || body.to_lowercase().contains("sending automated queries");
    (wall && !body.contains("results-standard"))
        .then_some("blocked the request with its automated-queries wall")
}

/// Click the ALTCHA checkbox once. Best-effort: the page may have no widget
/// (results were served directly) and a bot-hardened renderer can hang
/// `Runtime.evaluate`, in which case the poll loop below is still the
/// authority.
fn solve_captcha(page: &Page) {
    let _ = page.evaluate(
        "document.querySelector('altcha-widget input[type=checkbox]')?.click()",
        Duration::from_secs(5),
    );
}

/// Organic results in document order: `ul.results-standard > li`, title in
/// `h2 > a.title`, preview text in `p.s`. Clustered sub-results share the
/// shape; rows without a title anchor (infoboxes, spelling suggestions) are
/// skipped by construction.
pub(super) fn parse_html(html: &str) -> Vec<SearchHit> {
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
        hits.push(SearchHit::new(
            LABEL,
            title,
            url,
            class_text(block, "s").unwrap_or_default(),
        ));
    }
    hits
}

pub fn search(ctx: &Ctx<'_>) -> anyhow::Result<Vec<SearchHit>> {
    let url = build_url(ctx);
    let direct = get(&url, Some(HOME));
    let plan = BrowserPlan {
        url: url.clone(),
        warmup: Some(HOME),
        prelude: Some(solve_captcha),
    };
    finish(ctx, LABEL, direct, plan, parse_html, is_blocked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::ai::config::BrowserConfig;

    const FIXTURE: &str = r#"
<ul class="results-standard">
  <li class="r1">
    <h2><a class="title" href="https://www.rust-lang.org/">Rust Programming Language</a></h2>
    <p class="s">A language empowering everyone to build reliable software.</p>
  </li>
  <li class="r2 clu-result">
    <h2><a class="title" href="https://doc.rust-lang.org/">The Rust Book</a></h2>
    <p class="s s-1">The official book.</p>
  </li>
  <li class="infobox"><p>no anchor here</p></li>
  <li class="r3">
    <h2><a class="title" href="https://www.mojeek.de/search?q=more">Next page</a></h2>
  </li>
</ul>
"#;

    #[test]
    fn parses_rows_and_skips_navigation_and_anchorless_rows() {
        let hits = parse_html(FIXTURE);
        assert_eq!(hits.len(), 2, "{hits:#?}");
        assert_eq!(hits[0].title, "Rust Programming Language");
        assert_eq!(hits[0].url, "https://www.rust-lang.org/");
        assert_eq!(
            hits[0].snippet,
            "A language empowering everyone to build reliable software."
        );
        // A clustered sub-result keeps the shape and the `s s-1` class list.
        assert_eq!(hits[1].url, "https://doc.rust-lang.org/");
        assert_eq!(hits[1].snippet, "The official book.");
    }

    #[test]
    fn pages_without_the_results_list_yield_nothing() {
        assert!(parse_html("<html><body>nothing</body></html>").is_empty());
        assert!(parse_html("").is_empty());
    }

    #[test]
    fn the_captcha_and_refusal_pages_are_walls() {
        assert!(is_blocked(200, SEARCH_URL, "<altcha-widget></altcha-widget>").is_some());
        assert!(is_blocked(403, SEARCH_URL, "sending automated queries").is_some());
        // A rendered results page that merely embeds the widget is fine.
        assert!(is_blocked(200, SEARCH_URL, "altcha-widget results-standard").is_none());
        assert!(is_blocked(200, SEARCH_URL, "<ul class=\"results-standard\">").is_none());
    }

    #[test]
    fn url_carries_the_recency_token_verbatim() {
        let browser = BrowserConfig::default();
        let parsed = crate::web::search::query::parse("rust site:tokio.rs");
        let ctx = Ctx {
            query: "rust site:tokio.rs",
            parsed: &parsed,
            limit: 7,
            recency: Some(super::super::Recency::Month),
            browser: &browser,
        };
        let url = build_url(&ctx);
        assert!(url.contains("t=7"), "{url}");
        assert!(url.contains("since=month"), "{url}");
        assert!(url.contains("site%3Atokio.rs"), "site: 原样透传: {url}");
    }
}
