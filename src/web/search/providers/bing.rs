//! Bing — direct HTTPS GET + static HTML parsing. No browser tier.
//!
//! Plain queries hit this path first: one request, ~1 s, ten organic results.
//! It is the chain's cheapest engine, and the only one MyPi added on its own
//! (omp has no Bing).
//!
//! The engine is handed **plain keywords**, never operators. `www.bing.com`
//! parses Google-style syntax natively, but from a CN egress IP it 302s into
//! `cn.bing.com` no matter which escape-hatch parameters or cookies ride along
//! (verified: ensearch/mkt/setmkt/cc/SRCHHPGUSR all ignored), and `cn.bing`
//! silently drops every operator — leaving the literal `site:github.com` in
//! the query as search words. `engine.rs` therefore keeps directive-bearing
//! queries off this engine entirely; constraints are enforced by the
//! post-filter.

use super::super::hit::{SearchHit, take_unique};
use super::super::http::encode;
use super::super::query::{PLAIN_QUERY_SYNTAX, format_query};
use super::{Ctx, get};
use crate::web::utils::html::{extract_attr, extract_between, strip_tags, unescape_entities};
use crate::web::utils::url::percent_decode;

pub const LABEL: &str = "Bing";
const SEARCH_URL: &str = "https://www.bing.com/search";

pub(super) fn build_url(ctx: &Ctx<'_>) -> String {
    let query = format_query(ctx.parsed, PLAIN_QUERY_SYNTAX);
    // `count` is Bing's result-size parameter; honored without login.
    format!("{SEARCH_URL}?q={}&count={}", encode(&query), ctx.limit)
}

fn unwrap_tracking_href(href: &str) -> String {
    let href = unescape_entities(href);
    if let Some(pos) = href.find("&u=a1") {
        let enc = &href[pos + 5..];
        let end = enc.find('&').unwrap_or(enc.len());
        let enc = &enc[..end];
        // Base64url, Bing pads with the leftover chars stripped. Decode
        // leniently; failure keeps the wrapper (a working redirect beats a
        // dropped result).
        if let Some(bytes) = base64url_decode(enc)
            && let Ok(s) = String::from_utf8(bytes)
            && s.starts_with("http")
        {
            return s;
        }
        return href;
    }
    if let Some(pos) = href.find("&u=http") {
        // Old wrapper shape: &u=<percent-encoded target>. Bing lowercases
        // the hex digits (%3a not %3A) — decode case-insensitively.
        let rest = &href[pos + 3..];
        let end = rest.find('&').unwrap_or(rest.len());
        return percent_decode(&rest[..end]);
    }
    href
}

fn base64url_decode(s: &str) -> Option<Vec<u8>> {
    // Minimal base64url (RFC 4648 §5) decoder — no external crate, ~15 lines.
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a') as u32 + 26),
            b'0'..=b'9' => Some((c - b'0') as u32 + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    for &c in bytes {
        let v = val(c)?;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Extract organic results from a Bing SERP body. Public for tests.
pub fn parse_bing_html(html: &str) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    // Split into b_algo blocks; the lookahead form keeps each block's bounds.
    let block_re = |hay: &str| -> Vec<(usize, usize)> {
        let mut spans = Vec::new();
        let pat = "<li class=\"b_algo";
        let mut from = 0usize;
        while let Some(start) = hay[from..].find(pat) {
            let s = from + start;
            // Block ends at the next `<li class="b_algo` or the results-list
            // close; a `</li>` cap is unreliable (nested lists exist).
            let next = hay[s + pat.len()..]
                .find(pat)
                .map(|n| s + pat.len() + n)
                .unwrap_or(hay.len());
            spans.push((s, next));
            from = next;
        }
        spans
    };

    for (s, e) in block_re(html) {
        let block = &html[s..e];

        // Title anchor: the first <h2><a … href="…">…</a>. Require the h2 so
        // sub-links inside a result (deep-link rows) never take its place.
        // The href lives on the inner <a>, not on the <h2> itself — two-step
        // extraction: h2 range first, then its opening anchor tag.
        let Some(h2) = extract_between(block, "<h2", "</h2>") else {
            continue;
        };
        let Some(a_open) = extract_between(&h2.inner, "<a", ">").map(|t| t.open_tag) else {
            continue;
        };
        let Some(href) = extract_attr(&a_open, "href") else {
            continue;
        };
        let title = strip_tags(&h2.inner);
        if title.is_empty() {
            continue;
        }
        let url = unwrap_tracking_href(&href);

        // Snippet: several markup generations exist; take the first that
        // yields non-empty text.
        let snippet = [
            "<p class=\"b_lineclamp",
            "<p class=\"b_algoSlug",
            "<p class=\"b_caption",
        ]
        .iter()
        .find_map(|marker| {
            let pos = block.find(marker)?;
            let seg = &block[pos..];
            let p = extract_between(seg, "<p", "</p>")?;
            let text = unescape_entities(strip_tags(&p.inner).trim());
            (!text.is_empty()).then_some(text)
        })
        .unwrap_or_default();

        hits.push(SearchHit::new(LABEL, title, url, snippet));
    }
    hits
}

/// Single-request direct tier. Bing has no browser tier: unlike the scraper
/// engines, a render would not change what the markup says — a soft block on
/// this egress is about the operator set, not about JavaScript.
pub fn search(ctx: &Ctx<'_>) -> anyhow::Result<Vec<SearchHit>> {
    let url = build_url(ctx);
    let page = get(&url, None)?;
    if !page.is_ok() {
        anyhow::bail!("{LABEL}: HTTP {}", page.status);
    }
    // 200 + no b_algo = soft block (consent/JS gate). Surface it so the chain
    // advances instead of reporting "no results".
    if !page.body.contains("b_algo") && !page.body.contains("b_no") {
        anyhow::bail!(
            "{LABEL}: soft-blocked the direct request (HTTP {})",
            page.status
        );
    }
    let hits = take_unique(parse_bing_html(&page.body), ctx.limit);
    if hits.is_empty() {
        anyhow::bail!("{LABEL}: no results");
    }
    Ok(hits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::ai::config::BrowserConfig;

    // Fixture pinned to the markup Bing served during live testing (2026-09).
    const FIXTURE: &str = r#"
<ol id="b_results">
<li class="b_algo" data-id="1">
  <h2><a href="https://rust-lang.org/" h="ID=SERP,5144.1">Rust Programming Language</a></h2>
  <div class="b_caption">
    <p class="b_lineclamp b_lineclamp3">A language empowering everyone to write reliable &amp; efficient software.</p>
  </div>
</li>
<li class="b_algo" data-id="2">
  <h2><a href="/ck/a?e=abc&amp;u=a1aHR0cHM6Ly9kb2MucnVzdC1sYW5nLm9yZy8&amp;ntb=1" h="ID=SERP,5146.1">The Rust Book</a></h2>
  <div class="b_caption">
    <p class="b_algoSlug">Learn Rust with the official book.</p>
  </div>
</li>
<li class="b_algo">
  <h2><a href="https://www.bing.com/ck/a?e=x&amp;u=http%3a%2f%2fold.example.com%2fpage">Old form</a></h2>
</li>
<li class="b_algo">
  <h2><a href="https://example.com/plain">Plain link</a></h2>
  <p class="b_caption">no marker paragraph</p>
</li>
</ol>"#;

    fn ctx_for<'a>(
        query: &'a str,
        parsed: &'a crate::web::search::query::StructuredQuery,
        browser: &'a BrowserConfig,
    ) -> Ctx<'a> {
        Ctx {
            query,
            parsed,
            limit: 8,
            recency: None,
            browser,
        }
    }

    #[test]
    fn parses_organic_results_from_bing_markup() {
        let hits = parse_bing_html(FIXTURE);
        assert_eq!(hits.len(), 4);
        assert_eq!(hits[0].title, "Rust Programming Language");
        assert_eq!(hits[0].url, "https://rust-lang.org/");
        assert_eq!(
            hits[0].snippet,
            "A language empowering everyone to write reliable & efficient software."
        );
        assert_eq!(hits[0].provider, "Bing");
    }

    #[test]
    fn decodes_the_click_tracking_wrapper() {
        let hits = parse_bing_html(FIXTURE);
        // base64url("https://doc.rust-lang.org/")
        assert_eq!(hits[1].url, "https://doc.rust-lang.org/");
        // Old &u=http… percent-encoded form
        assert_eq!(hits[2].url, "http://old.example.com/page");
        assert_eq!(hits[3].url, "https://example.com/plain");
    }

    #[test]
    fn snippets_survive_markup_and_entities() {
        let hits = parse_bing_html(FIXTURE);
        assert_eq!(hits[1].snippet, "Learn Rust with the official book.");
    }

    #[test]
    fn empty_body_yields_no_hits() {
        assert!(parse_bing_html("<ol id=\"b_results\"></ol>").is_empty());
        assert!(parse_bing_html("").is_empty());
        assert!(parse_bing_html("consent page, nothing here").is_empty());
    }

    #[test]
    fn operators_never_reach_the_engine() {
        // Bing on this egress drops operators; sending them would leave the
        // literal `site:…` in the query as search words.
        let parsed = crate::web::search::query::parse("site:github.com \"async runtime\" -tokio");
        let browser = BrowserConfig::default();
        let ctx = ctx_for(
            "site:github.com \"async runtime\" -tokio",
            &parsed,
            &browser,
        );
        let url = build_url(&ctx);
        assert!(!url.contains("site%3A"), "{url}");
        assert!(!url.contains("-%22") && !url.contains("-tokio"), "{url}");
        assert!(url.contains("count=8"), "{url}");
        // The terms themselves survive.
        assert!(url.contains("async"), "{url}");
    }

    #[test]
    fn base64url_decoder_matches_known_vectors() {
        assert_eq!(
            base64url_decode("aHR0cHM6Ly9leGFtcGxlLmNvbS8"),
            Some(b"https://example.com/".to_vec())
        );
        assert_eq!(base64url_decode(""), None);
    }
}
