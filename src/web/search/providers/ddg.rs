//! DDG engine — html.duckduckgo.com rendered through the browser session.
//!
//! Advanced-syntax queries land here: the browser honors `site:`/quoted
//! terms exactly, and the shared session keeps cookies warm so the
//! anomaly wall stays quiet. Parsing is pure (`parse_ddg_html`), the
//! browser part is a thin driver over `session::page`.

use anyhow::anyhow;

use super::super::engine::SearchHit;
use crate::ai::config::BrowserConfig;
use crate::web::utils::html::{extract_attr, extract_between, strip_tags, unescape_entities};
use crate::web::utils::session::{self, RENDER_WAIT};
use crate::web::utils::url::{percent_decode, urlencoded};

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
        let Some(seg_end) = html[anchor_pos..].find("</a>") else {
            break;
        };
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

        hits.push(SearchHit {
            title,
            url: unwrap_ddg_href(&href),
            snippet,
        });
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

pub(crate) fn via_browser(
    query: &str,
    limit: usize,
    cfg: &BrowserConfig,
) -> anyhow::Result<Vec<SearchHit>> {
    let url = format!("{DDG_URL}?q={}", urlencoded(query));
    // A transient tab is opened straight at the SERP (create_target does
    // the navigation). `wait` polls the rendered DOM until `ready` says
    // stop; the anomaly wall aborts early instead of burning the budget.
    let html = session::with_transient(cfg, &url, |p| {
        p.wait(RENDER_WAIT, |html| {
            if html.contains("anomaly") {
                Some(Err(anyhow!(
                    "ddg anomaly-walled the browser session; retry later"
                )))
            } else if html.contains("result__a") && !parse_ddg_html(html).is_empty() {
                Some(Ok(()))
            } else {
                None
            }
        })
    })?;
    Ok(parse_ddg_html(&html).into_iter().take(limit).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
