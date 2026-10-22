//! Startpage — Google-backed results behind a privacy frontend, ported from
//! oh-my-pi's `startpage.ts`.
//!
//! The page itself is server-rendered with no JS challenge on the happy path.
//! The bot defense keys on requests that skip the homepage handshake: the
//! search form carries a session token (`sc`) plus sibling hidden inputs, and
//! posting with a stale or absent token 302s into the `/errors/` CAPTCHA
//! shell. So the direct tier performs the same dance a real browser does — GET
//! the homepage, lift the form's hidden inputs, POST them back with the query
//! — and degrades to a tokenless GET when the form cannot be read.

use super::super::hit::SearchHit;
use super::super::http::{self, HtmlPage};
use super::super::query::{GOOGLE_QUERY_SYNTAX, format_scraper_query};
use super::{BrowserPlan, Ctx, absolute, finish, is_external};
use crate::web::utils::html::{
    class_blocks, class_element, class_text, extract_attr, form_with, hidden_inputs, open_tag_at,
    text_at,
};

pub const LABEL: &str = "Startpage";
const HOME: &str = "https://www.startpage.com/";
const SEARCH_URL: &str = "https://www.startpage.com/sp/search";

pub(super) fn build_url(query: &str, with_date: Option<&str>) -> String {
    let mut url = format!("{SEARCH_URL}?query={}", http::encode(query));
    if let Some(date) = with_date {
        url.push_str("&with_date=");
        url.push_str(date);
    }
    url
}

/// Rejected requests 302 to `/en/errors/` (legacy `/sp/captcha`), a Gatsby SPA
/// whose chunk map names the captcha components. A bare "captcha" substring is
/// deliberately not used — snippets for captcha-related queries would
/// false-positive.
///
/// Startpage also fronts the whole site with Anubis, a proof-of-work bot wall,
/// from shared/datacenter egress: the homepage itself comes back as the
/// challenge, so there is no search form to lift and no result markup to
/// parse. Recognized separately because the actionable difference is "no
/// amount of retrying helps from this IP" versus "retry later".
pub(super) fn is_blocked(_status: u16, url: &str, body: &str) -> Option<&'static str> {
    if body.contains("anubis_challenge") || body.contains("anubis_version") {
        return Some("blocked the request with a proof-of-work challenge (Anubis)");
    }
    let challenged = url.contains("/errors/")
        || url.contains("/captcha/")
        || url.contains("/sp/captcha")
        || body.contains("component---src-pages-captcha")
        || body.contains("/sp/captcha");
    challenged.then_some("blocked the request with a CAPTCHA challenge")
}

/// Lift the hidden inputs from the homepage's `/sp/search` form. `None` when
/// the form or its `sc` anti-bot token is missing, so the caller degrades to a
/// tokenless GET instead of posting a doomed form.
fn form_inputs(body: &str) -> Option<Vec<(String, String)>> {
    let form = form_with(body, "action=\"/sp/search\"")?;
    let inputs = hidden_inputs(&form);
    inputs
        .iter()
        .any(|(name, _)| name == "sc")
        .then_some(inputs)
}

fn direct_tier(query: &str, with_date: Option<&str>) -> anyhow::Result<HtmlPage> {
    let inputs = match http::get(HOME, None) {
        Ok(page) if page.is_ok() && is_blocked(page.status, &page.url, &page.body).is_none() => {
            form_inputs(&page.body)
        }
        _ => None,
    };
    match inputs {
        Some(mut form) => {
            form.push(("query".to_string(), query.to_string()));
            if let Some(date) = with_date {
                form.push(("with_date".to_string(), date.to_string()));
            }
            http::post_form(SEARCH_URL, &form, Some(HOME))
        }
        None => http::get(&build_url(query, with_date), Some(HOME)),
    }
}

/// Organic results in document order: `div.result` blocks whose title anchor
/// `a.result-link` carries a direct target URL. The offscreen adblock honeypot
/// uses the class token `a-bg-result`, which whole-token class matching
/// ignores, and sponsored placements render outside `div.result`.
pub(super) fn parse_html(html: &str) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    for block in class_blocks(html, "result") {
        let Some(anchor) = class_element(block, "result-link") else {
            continue;
        };
        let Some(href) = open_tag_at(block, anchor).and_then(|tag| extract_attr(tag, "href"))
        else {
            continue;
        };
        let Some(url) = absolute(&href, HOME) else {
            continue;
        };
        if !is_external(&url, &["startpage.com"]) {
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
            class_text(block, "description").unwrap_or_default(),
        ));
    }
    hits
}

pub fn search(ctx: &Ctx<'_>) -> anyhow::Result<Vec<SearchHit>> {
    let query = format_scraper_query(ctx.query, Some(ctx.parsed), GOOGLE_QUERY_SYNTAX);
    let with_date = ctx.recency.map(|r| r.letter().to_string());
    let direct = direct_tier(&query, with_date.as_deref());
    let plan = BrowserPlan {
        // The browser tier repeats the handshake naturally (it loads the home
        // page first), so the tokenless URL is enough here.
        url: build_url(&query, with_date.as_deref()),
        warmup: Some(HOME),
        prelude: None,
    };
    finish(ctx, LABEL, direct, plan, parse_html, is_blocked)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"
<div class="a-bg-result"><a class="result-link" href="https://honeypot.test/"><h2 class="wgl-title">Do not show</h2></a></div>
<div class="result">
  <a class="result-link" href="https://doc.rust-lang.org/book/"><h2 class="wgl-title">The Rust Book</h2></a>
  <p class="description">The official book, free and online.</p>
</div>
<div class="result">
  <a class="result-link" href="https://www.startpage.com/sp/search?query=more"><h3>Next page</h3></a>
</div>
<div class="result">
  <a class="result-link" href="https://tokio.rs/"><h2>Tokio</h2></a>
</div>
"#;

    #[test]
    fn parses_results_and_skips_the_honeypot_and_self_links() {
        let hits = parse_html(FIXTURE);
        assert_eq!(hits.len(), 2, "{hits:#?}");
        assert_eq!(hits[0].title, "The Rust Book");
        assert_eq!(hits[0].url, "https://doc.rust-lang.org/book/");
        assert_eq!(hits[0].snippet, "The official book, free and online.");
        // A result with no description keeps an empty snippet rather than
        // borrowing the neighbour's.
        assert_eq!(hits[1].url, "https://tokio.rs/");
        assert_eq!(hits[1].snippet, "");
    }

    #[test]
    fn form_inputs_requires_the_anti_bot_token() {
        let with_token = r#"<form action="/sp/search"><input type="hidden" name="sc" value="tok"><input type="hidden" name="qid" value="1"><input name="query"></form>"#;
        let inputs = form_inputs(with_token).expect("有 sc 就该取到表单");
        assert_eq!(inputs.len(), 2);
        assert!(inputs.iter().any(|(n, v)| n == "sc" && v == "tok"));

        // No `sc`: posting would 302 into the CAPTCHA shell, so give up and
        // let the caller use the tokenless GET.
        let no_token =
            r#"<form action="/sp/search"><input type="hidden" name="qid" value="1"></form>"#;
        assert!(form_inputs(no_token).is_none());
        assert!(form_inputs("<html></html>").is_none());
    }

    #[test]
    fn captcha_shells_are_walls() {
        assert!(is_blocked(200, "https://www.startpage.com/en/errors/", "").is_some());
        assert!(is_blocked(200, SEARCH_URL, "component---src-pages-captcha").is_some());
        assert!(is_blocked(200, SEARCH_URL, "<div class=\"result\">").is_none());
        // A result snippet mentioning captchas is not a wall.
        assert!(is_blocked(200, SEARCH_URL, "how to solve captcha challenges").is_none());
        // Anubis is recognized as its own wall (the homepage itself comes back
        // as the challenge from shared egress).
        let anubis = r#"<script id="anubis_challenge" type="application/json">{"rules":{"difficulty":6}}</script>"#;
        assert_eq!(
            is_blocked(200, HOME, anubis),
            Some("blocked the request with a proof-of-work challenge (Anubis)")
        );
    }

    #[test]
    fn urls_carry_the_query_and_the_date_window() {
        assert_eq!(
            build_url("rust site:tokio.rs", None),
            format!("{SEARCH_URL}?query=rust%20site%3Atokio.rs")
        );
        assert!(build_url("rust", Some("w")).ends_with("&with_date=w"));
    }
}
