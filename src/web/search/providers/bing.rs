//! Bing engine — direct HTTPS GET + static HTML parsing. No browser.
//!
//! Plain queries hit this path first (verified live: ~1 s, 10 organic
//! results). The CN egress serves `li.b_algo` markup that still honors
//! nothing beyond plain terms — advanced-syntax queries route to the DDG
//! engine instead (see `super::ddg`).

use anyhow::{Context as _, anyhow};
use std::time::Duration;

use super::super::engine::SearchHit;
use crate::web::utils::html::{extract_attr, extract_between, strip_tags, unescape_entities};
use crate::web::utils::url::{percent_decode, urlencoded};

pub(super) const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36";
const SEARCH_URL: &str = "https://www.bing.com/search";
pub(super) const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

fn search_url(query: &str, limit: usize) -> String {
    // `count` is Bing's result-size parameter; honored without login.
    format!("{SEARCH_URL}?q={}&count={}", urlencoded(query), limit)
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
        return href.to_string();
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

        hits.push(SearchHit {
            title,
            url,
            snippet,
        });
    }
    hits
}

// --- Tier 1: direct HTTPS ----------------------------------------------------

pub(crate) fn direct(query: &str, limit: usize) -> anyhow::Result<Vec<SearchHit>> {
    let url = search_url(query, limit);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        .build()
        .into();
    let mut response = agent
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .header("Accept-Language", "en-US,en;q=0.9")
        .call()
        .map_err(|e| anyhow!("bing request failed: {e}"))?;

    let status = response.status().as_u16();
    let body = response
        .body_mut()
        .read_to_string()
        .context("reading bing body")?;

    // 200 + no b_algo = soft block (consent/JS gate). Surface it so the
    // caller falls back instead of returning "no results".
    if status != 200 || (!body.contains("b_algo") && !body.contains("b_no")) {
        anyhow::bail!("bing soft-blocked the direct request (HTTP {status})");
    }
    Ok(parse_bing_html(&body))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn query_passes_through_url_encoded() {
        let url = search_url("site:github.com \"async runtime\" -tokio", 8);
        assert!(url.starts_with("https://www.bing.com/search?q=site"));
        assert!(url.contains("count=8"));
        // The dead mkt/ensearch escape hatches stay out: verified no-ops on
        // CN egress, and their presence suggests a guarantee we don't have.
        assert!(!url.contains("mkt="));
        assert!(!url.contains("ensearch"));
        // quote/space encoding sanity
        assert!(url.contains("%22async%20runtime%22"));
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
