//! Web search — Bing HTML scraping with an engine chain, no API keys.
//!
//! Tier 1: plain HTTPS GET against `bing.com/search` (static HTML, no JS
//! needed — verified live: 10 organic results, ~1.3 s). Tier 2: headless
//! Helium via the CDP layer when the direct hit is bot-walled (302 to a
//! consent page, JS challenge, empty body). Both tiers parse the same
//! markup (`li.b_algo` blocks), so the extraction lives in one place.
//!
//! Deliberately single-engine by default: Bing's organic slots are
//! ad-free in practice (sponsored entries use distinct classes outside
//! `b_algo`), and uBlock in the bundled Helium profile strips the rest.
//! A Google fallback can slot into the chain later without touching the
//! tool surface.

use anyhow::{Context as _, anyhow};
use serde::Deserialize;
use std::time::{Duration, Instant};

// Chromium-family UA: Bing serves the full desktop result markup to it.
// A curl/empty UA gets a JS-gated page whose body has no `b_algo` at all.
const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36";
const SEARCH_URL: &str = "https://www.bing.com/search";
const DEFAULT_LIMIT: usize = 8;
const MAX_LIMIT: usize = 20;
const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

// One organic hit. `snippet` is the two-line preview under the title; a
// missing snippet still leaves a usable link row.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub snippet: String,
}

#[derive(Debug, Deserialize)]
pub struct SearchArgs {
    /// 一句话说明这次调用要干什么，中文，会显示给用户看
    pub intent: String,
    /// 搜索词，支持 site: -词 "短语" filetype: inurl: intitle: OR 等高级语法
    pub query: String,
    /// 返回条数，默认 8，上限 20
    pub limit: Option<usize>,
}

/// Parse arguments exactly as the model sent them (already JSON string).
pub fn parse_search_args(arguments: &str) -> anyhow::Result<SearchArgs> {
    serde_json::from_str(arguments).map_err(|e| anyhow!("bad search args: {e}"))
}

// Query assembly for the **direct tier**. Bing parses Google-style operators
// natively (site:, quotes, negation, OR, filetype:, inurl:, intitle:) —
// **except from a CN egress IP**, where www.bing.com 302s into cn.bing.com
// no matter which escape-hatch parameters or cookies ride along (verified:
// ensearch/mkt/setmkt/cc/SRCHHPGUSR all ignored) and cn.bing silently drops
// every operator. The direct tier therefore serves plain-keyword queries
// only; syntax-bearing queries route to the DDG browser tier, which honors
// them all.
fn search_url(query: &str, limit: usize) -> String {
    // `count` is Bing's result-size parameter; honored without login.
    format!("{SEARCH_URL}?q={}&count={}", urlencoded(query), limit)
}

fn urlencoded(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

// --- HTML extraction ---------------------------------------------------------
//
// Bing's organic results are `<li class="b_algo">` blocks. Inside each:
//   title  → first `<h2><a href="…">title</a></h2>`
//   url    → that anchor's href (already absolute; CN builds sometimes wrap
//            through bing.com/ck/a — decoded below)
//   snippet→ `<p class="b_lineclamp …">` or `<p class="b_algoSlug">`
//
// Regex, not a DOM parser: the block structure is flat and stable, and this
// crate already depends on nothing heavier than ureq. The tests pin the
// shapes that matter.

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    for ch in s.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            c if depth == 0 => out.push(c),
            _ => {}
        }
    }
    // Collapse whitespace runs (titles embed <strong> markers between words).
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn unescape_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

// Decode Bing's click-tracking wrapper: /ck/a?…&u=a1<base64url-target>
// (and the older &u={target} shape). Returns the input unchanged when the
// href is a plain link.
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
                && s.starts_with("http") {
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

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
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
        let snippet = ["<p class=\"b_lineclamp", "<p class=\"b_algoSlug", "<p class=\"b_caption"]
            .iter()
            .find_map(|marker| {
                let pos = block.find(marker)?;
                let seg = &block[pos..];
                let p = extract_between(seg, "<p", "</p>")?;
                let text = unescape_entities(strip_tags(&p.inner).trim());
                (!text.is_empty()).then_some(text)
            })
            .unwrap_or_default();

        hits.push(SearchHit { title, url, snippet });
    }
    hits
}

struct Tag {
    open_tag: String,
    inner: String,
}

fn extract_between(hay: &str, open_prefix: &str, close: &str) -> Option<Tag> {
    let start = hay.find(open_prefix)?;
    let gt = hay[start..].find('>')? + start;
    let open_tag = hay[start..=gt].to_string();
    let inner_start = gt + 1;
    let end = hay[inner_start..].find(close)? + inner_start;
    Some(Tag { open_tag, inner: hay[inner_start..end].to_string() })
}

fn extract_attr(tag: &str, attr: &str) -> Option<String> {
    // href="…" or href='…'
    for q in ['"', '\''] {
        let pat = format!("{attr}={q}");
        if let Some(pos) = tag.find(&pat) {
            let rest = &tag[pos + pat.len()..];
            let end = rest.find(q)?;
            return Some(rest[..end].to_string());
        }
    }
    None
}

// --- Tier 1: direct HTTPS ----------------------------------------------------

fn bing_direct(query: &str, limit: usize) -> anyhow::Result<Vec<SearchHit>> {
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

/// Detect query operators Bing drops on a CN egress (verified live: cn.bing
/// silently ignores site:, quotes, negation, filetype:, inurl:, intitle:).
/// A query carrying any of them must not go through the direct tier.
fn has_advanced_syntax(query: &str) -> bool {
    // Quoted phrase or negation: unambiguous operators on every engine.
    if query.contains('"') {
        return true;
    }
    // Operator whitelist — exactly the tokens DDG (and Google-style engines)
    // define. Deliberately NOT "any word followed by :": a plain-words query
    // like `hello:world` would otherwise be forced onto the slow browser
    // tier for no benefit.
    const OPERATORS: [&str; 8] =
        ["site", "inurl", "intitle", "intext", "filetype", "ext", "before", "after"];
    for token in query.split_whitespace() {
        if token.starts_with('-')
            && token.len() > 1
            && token[1..].contains(|c: char| c.is_ascii_alphanumeric())
        {
            return true; // negation: -tokio
        }
        let Some((head, tail)) = token.split_once(':') else { continue };
        if OPERATORS.contains(&head) && !tail.is_empty() {
            return true;
        }
    }
    false
}

// --- DDG browser engine ------------------------------------------------------
//
// html.duckduckgo.com is DDG's no-JS frontend: static HTML, full operator
// support, and (verified live) reachable only through a real browser session
// from this network — direct HTTP gets anomaly-walled regardless of UA/POST.
// It is the syntax-complete engine; the Bing direct tier is the fast path.

const DDG_URL: &str = "https://html.duckduckgo.com/html/";

/// Parse a DDG html-frontend SERP. Public for tests. Blocks are
/// `div.result` containers; title anchor `a.result__a` carries the target
/// URL inside `//duckduckgo.com/l/?uddg=<encoded>` (unwrap below), and the
/// snippet sits in `a|div.result__snippet`.
pub fn parse_ddg_html(html: &str) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    let marker = "result__a";
    let mut from = 0usize;
    while let Some(rel) = html[from..].find(marker) {
        let anchor_pos = from + rel;
        // Title anchor's href + text.
        let Some(seg_end) = html[anchor_pos..].find("</a>") else { break };
        let seg = &html[anchor_pos..anchor_pos + seg_end];
        let Some(open_end) = seg.find('>') else { break };
        let Some(href) = extract_attr(&seg[..open_end], "href") else {
            from = anchor_pos + marker.len();
            continue;
        };
        let title = strip_tags(&seg[open_end..]);
        if title.is_empty() {
            from = anchor_pos + marker.len();
            continue;
        }

        // Snippet: the next result__snippet before the following anchor.
        let next_anchor = html[anchor_pos + seg.len()..]
            .find(marker)
            .map(|n| anchor_pos + seg.len() + n)
            .unwrap_or(html.len());
        let tail = &html[anchor_pos + seg.len()..next_anchor];
        let snippet = tail
            .find("result__snippet")
            .and_then(|pos| extract_between(&tail[pos..], ">", "</a>").map(|t| t.inner))
            .map(|s| strip_tags(&s))
            .unwrap_or_default();

        hits.push(SearchHit { title, url: unwrap_ddg_href(&href), snippet });
        from = next_anchor;
    }
    hits
}

// DDG wraps outbound links as //duckduckgo.com/l/?uddg=<percent-encoded> —
// unwrap to the real target; leave everything else as-is.
fn unwrap_ddg_href(href: &str) -> String {
    let href = unescape_entities(href);
    if let Some(pos) = href.find("uddg=") {
        let enc = &href[pos + 5..];
        let end = enc.find('&').unwrap_or(enc.len());
        let decoded = percent_decode(&enc[..end]);
        if decoded.starts_with("http") {
            return decoded;
        }
    }
    if let Some(rest) = href.strip_prefix("//") {
        return format!("https:{rest}");
    }
    href
}

fn ddg_via_browser(query: &str, limit: usize) -> anyhow::Result<Vec<SearchHit>> {
    let profile = crate::xdg::browser_profile_dir();
    // Long-lived mode: MYPI_BROWSER_PORT points at a browser someone keeps
    // running (session warmth matters to anti-bot frontends). Otherwise
    // spawn a headless one for this call and reap it on the way out.
    let browser = match std::env::var("MYPI_BROWSER_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
    {
        Some(port) => crate::cdp::Browser::attach(&profile, port)
            .context("attaching to MYPI_BROWSER_PORT")?,
        None => crate::cdp::Browser::launch(&profile)
            .context("launching browser for search")?,
    };
    let mut target = browser.page_target().context("no page target")?;
    let mut cdp = crate::cdp::Cdp::connect(&target.ws_url)?;

    let url = format!("{DDG_URL}?q={}", urlencoded(query));
    cdp.navigate(&url, NAVIGATE_TIMEOUT)?;

    // Poll the rendered DOM. Cross-document navigation can reset the
    // per-target socket mid-flight — a failed poll re-resolves the target
    // (the tab id survives) and reconnects once, transparently.
    let deadline = Instant::now() + RENDER_WAIT;
    loop {
        let html = match cdp.dom_html(POLL_TIMEOUT) {
            Ok(html) => html,
            Err(e) if e.to_string().contains("connection reset") => {
                target = browser.page_target().context("no page target")?;
                cdp = crate::cdp::Cdp::connect(&target.ws_url)?;
                continue;
            }
            Err(e) => return Err(e),
        };
        if html.contains("result__a") {
            let hits = parse_ddg_html(&html);
            if !hits.is_empty() {
                return Ok(hits.into_iter().take(limit).collect());
            }
        }
        if html.contains("anomaly") {
            return Err(anyhow!(
                "ddg anomaly-walled the browser session; retry later"
            ));
        }
        if Instant::now() > deadline {
            return Err(anyhow!("ddg served no results in the browser"));
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(20);
const RENDER_WAIT: Duration = Duration::from_secs(15);
const POLL_TIMEOUT: Duration = Duration::from_secs(8);

pub fn search(args: &SearchArgs) -> anyhow::Result<Vec<SearchHit>> {
    let limit = args.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);

    // Syntax-bearing queries bypass Bing entirely (CN egress drops every
    // operator — verified); plain keywords take the fast direct tier first.
    if !has_advanced_syntax(&args.query) {
        match bing_direct(&args.query, limit) {
            Ok(hits) if !hits.is_empty() => return Ok(hits),
            Ok(_) => {}
            Err(_) => {}
        }
    }
    ddg_via_browser(&args.query, limit)
}

/// Tool-facing text rendering: numbered rows, snippet on its own line. The
/// engine name is deliberately absent — the model doesn't need it and it
/// would leak into answers.
pub fn render(hits: &[SearchHit]) -> String {
    if hits.is_empty() {
        return "no results".to_string();
    }
    let mut out = String::new();
    for (i, h) in hits.iter().enumerate() {
        out.push_str(&format!("{}. {}\n   {}\n", i + 1, h.title, h.url));
        if !h.snippet.is_empty() {
            out.push_str(&format!("   {}\n", h.snippet));
        }
        out.push('\n');
    }
    out
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
        assert_eq!(hits[0].snippet, "A language empowering everyone to write reliable & efficient software.");
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
    fn render_is_numbered_and_has_no_engine_name() {
        let hits = vec![SearchHit {
            title: "T".into(),
            url: "https://x/".into(),
            snippet: "s".into(),
        }];
        let out = render(&hits);
        assert!(out.starts_with("1. T\n"));
        assert!(!out.to_lowercase().contains("bing"));
    }

    #[test]
    fn base64url_decoder_matches_known_vectors() {
        assert_eq!(
            base64url_decode("aHR0cHM6Ly9leGFtcGxlLmNvbS8"),
            Some(b"https://example.com/".to_vec())
        );
        assert_eq!(base64url_decode(""), None);
    }
    // --- DDG engine & routing -------------------------------------------

    const DDG_FIXTURE: &str = r#"
<div class="links_main deep">
  <h2 class="result__title">
    <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fgithub.com%2Frust-lang%2Fasync-book&amp;rut=abc">Asynchronous Programming In Rust</a>
  </h2>
  <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fgithub.com%2Frust-lang%2Fasync-book">Learn async <b>Rust</b> with the async book.</a>
</div>
<div class="links_main deep">
  <h2 class="result__title">
    <a rel="nofollow" class="result__a" href="https://smol.rs/">smol</a>
  </h2>
  <a class="result__snippet" href="https://smol.rs/">A small and fast async runtime.</a>
</div>"#;

    #[test]
    fn parses_ddg_results_and_unwraps_uddg() {
        let hits = parse_ddg_html(DDG_FIXTURE);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].title, "Asynchronous Programming In Rust");
        assert_eq!(hits[0].url, "https://github.com/rust-lang/async-book");
        assert_eq!(hits[0].snippet, "Learn async Rust with the async book.");
        // Plain (non-wrapped) href passes through.
        assert_eq!(hits[1].url, "https://smol.rs/");
        assert_eq!(hits[1].snippet, "A small and fast async runtime.");
    }

    #[test]
    fn syntax_detection_routes_by_operator() {
        // Operators Bing-on-CN-egress drops:
        assert!(has_advanced_syntax("site:github.com rust"));
        assert!(has_advanced_syntax("rust -tokio"));
        assert!(has_advanced_syntax("\"async runtime\""));
        assert!(has_advanced_syntax("filetype:pdf rust book"));
        // A colon inside a URL-ish word is NOT an operator (no operator head).
        assert!(!has_advanced_syntax("rust async book"));
        assert!(!has_advanced_syntax("hello:world-with-dashes"));
        // Lone dash or colon is punctuation, not an operator.
        assert!(!has_advanced_syntax("rust - "));
        assert!(!has_advanced_syntax("rust :"));
    }
}
