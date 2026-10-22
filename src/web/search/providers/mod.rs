//! Search engines — one module per engine, plus the shared two-tier fetch
//! contract they all follow.
//!
//! Each engine implements the same shape:
//!
//! - `build_url` / a form builder: the request a real browser would make,
//! - `parse_html`: a pure, fixture-testable HTML scan,
//! - `is_blocked`: recognize the site's bot wall from the response body,
//! - `search`: direct HTTP first, browser rendering only when the direct
//!   answer is a wall (or a transport error) — omp's `browserFetch`.
//!
//! Routing, chaining and merging live in `engine.rs`; these modules know
//! nothing about each other.

pub(super) mod bing;
pub(super) mod duckduckgo;
pub(super) mod ecosia;
pub(super) mod google;
pub(super) mod mojeek;
pub(super) mod startpage;

use anyhow::anyhow;
use serde::Deserialize;

use super::hit::{SearchHit, take_unique};
use super::http::{self, HtmlPage};
use super::query::StructuredQuery;
use crate::server::ai::config::BrowserConfig;
use crate::web::utils::session::{self, Page, RENDER_WAIT};

/// Temporal filter narrowing results to a window. Engines that expose no date
/// filter ignore it — neither the query nor the ranking may be rewritten to
/// approximate one (omp's `recency` contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Recency {
    Day,
    Week,
    Month,
    Year,
}

impl Recency {
    /// The single-letter token most of these engines accept.
    pub(super) fn letter(self) -> &'static str {
        match self {
            Recency::Day => "d",
            Recency::Week => "w",
            Recency::Month => "m",
            Recency::Year => "y",
        }
    }

    /// The word form Mojeek's `since` filter takes.
    pub(super) fn word(self) -> &'static str {
        match self {
            Recency::Day => "day",
            Recency::Week => "week",
            Recency::Month => "month",
            Recency::Year => "year",
        }
    }
}

/// Everything an engine needs for one query.
pub(super) struct Ctx<'a> {
    /// The raw query as the model wrote it.
    pub query: &'a str,
    pub parsed: &'a StructuredQuery,
    pub limit: usize,
    pub recency: Option<Recency>,
    pub browser: &'a BrowserConfig,
}

/// Recognize a bot wall: `Some(reason)` means "this body is not results".
/// Takes the response status, the URL it landed on, and the body, because the
/// three engines signal differently (status, redirect target, body markers).
pub(super) type Blocker = fn(u16, &str, &str) -> Option<&'static str>;

/// The browser half of an engine: which URL to render, and whether the
/// session should warm up on the site's home page first (Startpage's form
/// token and Google's consent cookie only exist after a home-page visit).
pub(super) struct BrowserPlan<'a> {
    pub url: String,
    pub warmup: Option<&'a str>,
    /// Optional page-side action before polling — Mojeek's ALTCHA checkbox.
    pub prelude: Option<fn(&Page)>,
}

/// Render `plan.url` through the shared browser session, polling the DOM
/// until `parse` sees results or `blocked` recognizes a wall.
pub(super) fn render(
    cfg: &BrowserConfig,
    plan: &BrowserPlan<'_>,
    label: &str,
    parse: fn(&str) -> Vec<SearchHit>,
    blocked: Blocker,
) -> anyhow::Result<String> {
    let start_at = plan.warmup.unwrap_or(&plan.url).to_string();
    let target = plan.url.clone();
    let html = session::with_transient(cfg, &start_at, |page| {
        if plan.warmup.is_some() {
            page.navigate(&target)?;
        }
        if let Some(prelude) = plan.prelude {
            prelude(page);
        }
        page.wait(RENDER_WAIT, |html| {
            if let Some(reason) = blocked(200, &target, html) {
                return Some(Err(anyhow!("{label}: {reason}")));
            }
            if !parse(html).is_empty() {
                Some(Ok(()))
            } else {
                None
            }
        })
    })?;
    if let Some(reason) = blocked(200, &target, &html) {
        anyhow::bail!("{label}: {reason}");
    }
    Ok(html)
}

/// What the direct tier produced: hits, or the reason it produced none.
/// Engines whose direct tier is a single request build this from the response;
/// engines that paginate (DDG) accumulate it in a loop.
pub(super) struct DirectOutcome {
    pub hits: Vec<SearchHit>,
    pub reason: Option<String>,
    /// Whether a render could plausibly do better. Transport errors, non-2xx
    /// responses and walls are worth a browser; an **empty but valid** SERP is
    /// not — the engine answered the query, it simply had nothing, and the
    /// chain moves on (this is omp's behavior: only a thrown provider error
    /// advances the chain, never a zero-result response).
    pub escalate: bool,
}

impl DirectOutcome {
    fn failed(reason: impl Into<String>) -> Self {
        Self {
            hits: Vec::new(),
            reason: Some(reason.into()),
            escalate: true,
        }
    }

    fn empty() -> Self {
        Self {
            hits: Vec::new(),
            reason: Some("no results".to_string()),
            escalate: false,
        }
    }

    fn hits(hits: Vec<SearchHit>) -> Self {
        Self {
            hits,
            reason: None,
            escalate: false,
        }
    }
}

/// The shared tail of every engine's `search`: return the direct hits if there
/// are any, otherwise render the same query in the browser, then parse and
/// dedup. A non-2xx status, a wall or a transport error escalates — the same
/// signals omp escalates on.
pub(super) fn finish_with(
    ctx: &Ctx<'_>,
    label: &str,
    direct: DirectOutcome,
    plan: BrowserPlan<'_>,
    parse: fn(&str) -> Vec<SearchHit>,
    blocked: Blocker,
) -> anyhow::Result<Vec<SearchHit>> {
    let DirectOutcome {
        hits,
        reason,
        escalate,
    } = direct;
    if !hits.is_empty() {
        return Ok(hits);
    }
    let reason = reason.unwrap_or_else(|| "no results".to_string());
    if !escalate {
        anyhow::bail!("{label}: {reason}")
    }

    match render(ctx.browser, &plan, label, parse, blocked) {
        Ok(html) => {
            let hits = take_unique(parse(&html), ctx.limit);
            if hits.is_empty() {
                anyhow::bail!("{label}: {reason}; browser: no results")
            }
            Ok(hits)
        }
        Err(e) => anyhow::bail!("{label}: {reason}; browser: {e}"),
    }
}

/// [`finish_with`] for the common single-request direct tier.
pub(super) fn finish(
    ctx: &Ctx<'_>,
    label: &str,
    direct: anyhow::Result<HtmlPage>,
    plan: BrowserPlan<'_>,
    parse: fn(&str) -> Vec<SearchHit>,
    blocked: Blocker,
) -> anyhow::Result<Vec<SearchHit>> {
    let outcome = match direct {
        Ok(page) => {
            if let Some(wall) = blocked(page.status, &page.url, &page.body) {
                DirectOutcome::failed(wall)
            } else if page.is_ok() {
                let hits = take_unique(parse(&page.body), ctx.limit);
                if hits.is_empty() {
                    DirectOutcome::empty()
                } else {
                    DirectOutcome::hits(hits)
                }
            } else {
                DirectOutcome::failed(format!("HTTP {}", page.status))
            }
        }
        Err(e) => DirectOutcome::failed(e.to_string()),
    };
    finish_with(ctx, label, outcome, plan, parse, blocked)
}

/// Reject non-http(s) targets and engine-internal navigation rows (verticals,
/// paging) that share the result markup.
pub(super) fn is_external(url: &str, engines: &[&str]) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return false;
    }
    let Some(host) = parsed.host_str() else {
        return false;
    };
    let host = host.to_lowercase();
    !engines
        .iter()
        .any(|e| host == *e || host.ends_with(&format!(".{e}")))
}

/// Absolute URL from a possibly relative href, resolved against `base`.
pub(super) fn absolute(href: &str, base: &str) -> Option<String> {
    if href.starts_with("http://") || href.starts_with("https://") {
        return Some(href.to_string());
    }
    if href.starts_with("//") {
        return Some(format!("https:{href}"));
    }
    let base = url::Url::parse(base).ok()?;
    base.join(href).ok().map(|u| u.to_string())
}

/// GET helper used by every engine's direct tier.
pub(super) fn get(url: &str, referer: Option<&str>) -> anyhow::Result<HtmlPage> {
    http::get(url, referer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_filters_engine_domains_and_bad_schemes() {
        assert!(is_external("https://target.test/page", &["mojeek.de"]));
        assert!(!is_external(
            "https://www.mojeek.de/search?q=x",
            &["mojeek.de"]
        ));
        assert!(!is_external(
            "https://mojeek.co.uk/x",
            &["mojeek.de", "mojeek.co.uk"]
        ));
        assert!(!is_external("javascript:void(0)", &["mojeek.de"]));
        assert!(!is_external("not a url", &["mojeek.de"]));
    }

    #[test]
    fn absolute_resolves_relative_and_protocol_relative_hrefs() {
        assert_eq!(
            absolute("//duckduckgo.com/x", "https://html.duckduckgo.com/").as_deref(),
            Some("https://duckduckgo.com/x")
        );
        assert_eq!(
            absolute("/url?q=x", "https://www.google.com/").as_deref(),
            Some("https://www.google.com/url?q=x")
        );
        assert_eq!(
            absolute("https://a.test/b", "https://www.google.com/").as_deref(),
            Some("https://a.test/b")
        );
    }
}
