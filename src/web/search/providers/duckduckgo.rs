//! DuckDuckGo — the no-JS HTML search frontend, ported from oh-my-pi's
//! `duckduckgo.ts`.
//!
//! A form POST to `html.duckduckgo.com/html/` returns a static results page
//! that parses without a browser, which is why this is the chain's general
//! purpose engine: it honors the full operator set except date bounds, it
//! publishes result dates, and it paginates through a continuation form.
//!
//! (omp tried the Instant Answer API first and documented why it was useless
//! for agent queries — it only answers Wikipedia/Wolfram-shaped topics. Live
//! verification agrees, so this engine is the HTML frontend only.)
//!
//! `before:`/`after:` are deliberately not forwarded: DDG does not parse them,
//! so they are left to the pipeline's lenient post-filter instead of being
//! emitted as literal noise.

use std::collections::HashSet;

use super::super::hit::{SearchHit, dedup_key};
use super::super::http;
use super::super::query::{QuerySyntax, format_scraper_query};
use super::{BrowserPlan, Ctx, DirectOutcome, absolute, finish_with, is_external};
use crate::web::utils::html::{
    class_blocks, class_element, extract_attr, form_with, hidden_inputs, open_tag_at, spans,
    text_at,
};

pub const LABEL: &str = "DuckDuckGo";
const HOME: &str = "https://html.duckduckgo.com/";
const SERP_URL: &str = "https://html.duckduckgo.com/html/";
/// Pages walked through the continuation form before the engine settles for
/// what it has. omp's equivalent bound is the shared 60 s deadline; a page
/// count is simpler and bounds the work deterministically.
const MAX_PAGES: usize = 3;

/// The operator set the DDG HTML frontend actually parses. Date bounds are
/// excluded — see the module comment.
const QUERY_SYNTAX: QuerySyntax = QuerySyntax {
    phrases: true,
    negation: true,
    or: true,
    site: true,
    in_url: true,
    in_title: true,
    in_text: true,
    filetype: true,
    date_range: false,
};

/// Recency → DDG's `df` form parameter (single letters).
fn recency_df(ctx: &Ctx<'_>) -> Option<&'static str> {
    ctx.recency.map(|r| r.letter())
}

pub(super) fn build_form(ctx: &Ctx<'_>) -> Vec<(String, String)> {
    let query = format_scraper_query(ctx.query, Some(ctx.parsed), QUERY_SYNTAX);
    let mut form = vec![
        ("q".to_string(), query),
        (
            "kl".to_string(),
            locale_to_kl(ctx.parsed.lang.as_deref())
                .unwrap_or("us-en")
                .to_string(),
        ),
    ];
    if let Some(df) = recency_df(ctx) {
        form.push(("df".to_string(), df.to_string()));
    }
    // The empty `b` field is what DDG's own form submits; omitting it is the
    // one shape difference the bot detector notices.
    form.push(("b".to_string(), String::new()));
    form
}

/// The GET form of the same query, for the browser tier (which cannot POST).
pub(super) fn build_url(ctx: &Ctx<'_>) -> String {
    let params: Vec<String> = build_form(ctx)
        .into_iter()
        .filter(|(_, v)| !v.is_empty())
        .map(|(k, v)| format!("{k}={}", http::encode(&v)))
        .collect();
    format!("{SERP_URL}?{}", params.join("&"))
}

/// DDG mixes status codes (200 vs 202) on its challenge pages, so the body is
/// the reliable signal.
pub(super) fn is_blocked(_status: u16, _url: &str, body: &str) -> Option<&'static str> {
    (body.contains("anomaly-modal") || body.contains("anomaly.js"))
        .then_some("blocked the request with a bot-detection challenge")
}

/// DuckDuckGo's documented `kl` values. The codes resemble `region-language`
/// locales but contain provider-specific identifiers (`jp-jp`, `tw-tzh`,
/// `uk-en`) that cannot be derived mechanically, so the allowlist is data.
const KL_CODES: [&str; 68] = [
    "xa-ar", "xa-en", "ar-es", "au-en", "at-de", "be-fr", "be-nl", "br-pt", "bg-bg", "ca-en",
    "ca-fr", "ct-ca", "cl-es", "cn-zh", "co-es", "hr-hr", "cz-cs", "dk-da", "ee-et", "fi-fi",
    "fr-fr", "de-de", "gr-el", "hk-tzh", "hu-hu", "in-en", "id-id", "id-en", "ie-en", "il-he",
    "it-it", "jp-jp", "kr-kr", "lv-lv", "lt-lt", "xl-es", "my-ms", "my-en", "mx-es", "nl-nl",
    "nz-en", "no-no", "pe-es", "ph-en", "ph-tl", "pl-pl", "pt-pt", "ro-ro", "ru-ru", "sg-en",
    "sk-sk", "sl-sl", "za-en", "es-es", "se-sv", "ch-de", "ch-fr", "ch-it", "tw-tzh", "th-th",
    "tr-tr", "ua-uk", "uk-en", "us-en", "ue-es", "ve-es", "vn-vi", "wt-wt",
];

/// BCP 47 locales whose DDG code is not a component swap.
fn kl_alias(locale: &str) -> Option<&'static str> {
    Some(match locale {
        "ca-es" => "ct-ca",
        "en-gb" => "uk-en",
        "es-419" => "xl-es",
        "es-us" => "ue-es",
        "ja-jp" => "jp-jp",
        "ko-kr" => "kr-kr",
        "zh-hk" => "hk-tzh",
        "zh-tw" => "tw-tzh",
        _ => return None,
    })
}

/// Map a parsed `lang:` locale onto DuckDuckGo's `kl` value. Shared queries use
/// `language-region` order while DDG uses `region-language`; anything that
/// does not survive the allowlist after swapping keeps the caller's default.
pub(super) fn locale_to_kl(lang: Option<&str>) -> Option<&'static str> {
    let lang = lang?;
    let locale = lang.to_lowercase().replace('_', "-");
    if let Some(alias) = kl_alias(&locale) {
        return Some(alias);
    }
    let bytes = locale.as_bytes();
    if bytes.len() != 5 || bytes[2] != b'-' {
        return None;
    }
    let candidate = format!("{}-{}", &locale[3..], &locale[..2]);
    KL_CODES.iter().copied().find(|code| *code == candidate)
}

/// Resolve a result href. DDG routes clicks through
/// `//duckduckgo.com/l/?uddg=<encoded>`, and mixes in protocol-relative and
/// absolute links, so all three shapes are accepted and anything else dropped.
fn unwrap_result_url(href: &str) -> Option<String> {
    let decoded = crate::web::utils::html::unescape_entities(href);
    if let Some(pos) = decoded.find("uddg=") {
        let rest = &decoded[pos + "uddg=".len()..];
        let end = rest.find('&').unwrap_or(rest.len());
        let target = crate::web::utils::url::percent_decode(&rest[..end]);
        if target.starts_with("http") {
            return Some(target);
        }
    }
    let url = absolute(decoded.as_str(), HOME)?;
    is_external(&url, &["duckduckgo.com"]).then_some(url)
}

/// Publication timestamp of a row. Recent DDG markup renders a bare
/// `<span>&nbsp; &nbsp; 2026-07-30T20:19:00.0000000</span>` inside
/// `result__extras__url`; restricting the scan to that container keeps a
/// date-shaped value in a snippet from being read as publication metadata.
fn extract_published_date(block: &str) -> Option<String> {
    let offset = class_element(block, "result__extras__url")?;
    let gt = block[offset..].find('>')? + offset;
    let end = block[gt..]
        .find("</div>")
        .map(|n| gt + n)
        .unwrap_or(block.len());
    spans(&block[offset..end])
        .into_iter()
        .map(|text| text.trim().to_string())
        .find(|text| is_iso_date_prefix(text))
}

fn is_iso_date_prefix(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && text[..4].bytes().all(|b| b.is_ascii_digit())
        && text[5..7].bytes().all(|b| b.is_ascii_digit())
        && text[8..10].bytes().all(|b| b.is_ascii_digit())
}

/// Hidden fields of DDG's next-page form, when the page offers one. Attribute
/// order varies across responses, so each input is read on its own.
fn continuation_form(body: &str) -> Option<Vec<(String, String)>> {
    let mut from = 0usize;
    while let Some(rel) = body[from..].find("<form") {
        let start = from + rel;
        let Some(form) = form_with(&body[start..], "") else {
            break;
        };
        let inputs = hidden_inputs(&form);
        if inputs.iter().any(|(n, _)| n == "s") && inputs.iter().any(|(n, _)| n == "vqd") {
            return Some(inputs);
        }
        from = start + 5;
    }
    None
}

/// Organic results in document order: `div.result` rows, title anchor
/// `a.result__a`, optional `result__snippet` sibling. Sponsored placements and
/// the trailing pagination row are tolerated, not special-cased.
pub(super) fn parse_html(html: &str) -> Vec<SearchHit> {
    let mut hits = Vec::new();
    for block in class_blocks(html, "result") {
        let Some(anchor) = class_element(block, "result__a") else {
            continue;
        };
        let Some(href) = open_tag_at(block, anchor).and_then(|tag| extract_attr(tag, "href"))
        else {
            continue;
        };
        let Some(url) = unwrap_result_url(&href) else {
            continue;
        };
        let title = text_at(block, anchor).unwrap_or_default();
        if title.is_empty() {
            continue;
        }
        let snippet = class_element(block, "result__snippet")
            .and_then(|offset| text_at(block, offset))
            .unwrap_or_default();
        hits.push(SearchHit::new(LABEL, title, url, snippet).dated(extract_published_date(block)));
    }
    hits
}

/// Walk the SERP through its continuation form, stopping at `limit`, at
/// [`MAX_PAGES`], or as soon as a page adds nothing new.
fn direct_tier(ctx: &Ctx<'_>) -> (Vec<SearchHit>, Option<String>) {
    let mut hits: Vec<SearchHit> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut form = Some(build_form(ctx));
    let mut pages = 0usize;
    let mut reason: Option<String> = None;

    while let Some(current) = form {
        if pages >= MAX_PAGES || hits.len() >= ctx.limit {
            break;
        }
        pages += 1;
        let page = match http::post_form(SERP_URL, &current, Some(HOME)) {
            Ok(page) => page,
            Err(e) => {
                reason = Some(e.to_string());
                break;
            }
        };
        if let Some(blocked) = is_blocked(page.status, &page.url, &page.body) {
            reason = Some(blocked.to_string());
            break;
        }
        if !page.is_ok() {
            reason = Some(format!("HTTP {}", page.status));
            break;
        }
        let before = hits.len();
        for hit in parse_html(&page.body) {
            if hits.len() >= ctx.limit {
                break;
            }
            if seen.insert(dedup_key(&hit.url)) {
                hits.push(hit);
            }
        }
        if hits.len() == before {
            break; // nothing new: the next page would repeat this one
        }
        form = continuation_form(&page.body);
    }
    (hits, reason)
}

pub fn search(ctx: &Ctx<'_>) -> anyhow::Result<Vec<SearchHit>> {
    let (hits, reason) = direct_tier(ctx);
    // A fully-fetched page set that simply had no hits is not a reason to
    // render: the engine answered, the chain should move on.
    let outcome = if !hits.is_empty() {
        DirectOutcome::hits(hits)
    } else {
        match reason {
            Some(reason) => DirectOutcome::failed(reason),
            None => DirectOutcome::empty(),
        }
    };
    let plan = BrowserPlan {
        url: build_url(ctx),
        warmup: None,
        prelude: None,
    };
    finish_with(ctx, LABEL, outcome, plan, parse_html, is_blocked)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"
<div class="result results_links results_links_deep web-result">
  <div class="links_main links_deep result__body">
    <h2 class="result__title"><a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdoc.rust-lang.org%2Fbook%2F&amp;rut=abc">The <b>Rust</b> Book</a></h2>
    <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdoc.rust-lang.org%2Fbook%2F">The official book &amp; guide.</a>
    <div class="result__extras"><div class="result__extras__url"><span>doc.rust-lang.org</span><span>&nbsp; &nbsp; 2026-07-30T20:19:00.0000000</span></div></div>
  </div>
</div>
<div class="result results_links">
  <h2 class="result__title"><a rel="nofollow" class="result__a" href="https://tokio.rs/">Tokio</a></h2>
  <div class="result__snippet">An asynchronous runtime.</div>
</div>
<div class="nav-link"><form action="/html/" method="post"><input type="hidden" name="q" value="rust"><input value="30" name="s" type="hidden"><input name="vqd" value="4-abc" type="hidden"></form></div>
"#;

    #[test]
    fn parses_rows_titles_snippets_and_dates() {
        let hits = parse_html(FIXTURE);
        assert_eq!(hits.len(), 2, "{hits:#?}");
        assert_eq!(hits[0].title, "The Rust Book");
        assert_eq!(hits[0].url, "https://doc.rust-lang.org/book/");
        assert_eq!(hits[0].snippet, "The official book & guide.");
        assert_eq!(
            hits[0].published_date.as_deref(),
            Some("2026-07-30T20:19:00.0000000")
        );
        // A row with a div snippet and no date.
        assert_eq!(hits[1].url, "https://tokio.rs/");
        assert_eq!(hits[1].snippet, "An asynchronous runtime.");
        assert_eq!(hits[1].published_date, None);
    }

    #[test]
    fn the_continuation_form_is_read_when_it_carries_both_fields() {
        let form = continuation_form(FIXTURE).expect("隐藏表单必须被找到");
        assert!(form.iter().any(|(n, v)| n == "s" && v == "30"));
        assert!(form.iter().any(|(n, v)| n == "vqd" && v == "4-abc"));
        // A form without `vqd` is not a continuation form.
        let other = r#"<form action="/"><input type="hidden" name="s" value="1"></form>"#;
        assert!(continuation_form(other).is_none());
    }

    #[test]
    fn anomaly_pages_are_walls() {
        assert!(is_blocked(202, SERP_URL, "<div id=\"anomaly-modal\">").is_some());
        assert!(is_blocked(200, SERP_URL, "anomaly.js").is_some());
        assert!(is_blocked(200, SERP_URL, "<div class=\"result\">").is_none());
    }

    #[test]
    fn kl_maps_locales_through_the_allowlist_and_aliases() {
        assert_eq!(locale_to_kl(Some("en")), None, "裸语言码不是地区");
        assert_eq!(locale_to_kl(Some("en-us")), Some("us-en"), "分量对调");
        assert_eq!(locale_to_kl(Some("en-gb")), Some("uk-en"), "别名优先");
        assert_eq!(locale_to_kl(Some("zh-tw")), Some("tw-tzh"));
        assert_eq!(locale_to_kl(Some("ja-jp")), Some("jp-jp"));
        assert_eq!(
            locale_to_kl(Some("pt-br")),
            Some("br-pt"),
            "白名单里有 br-pt 就得认"
        );
        assert_eq!(
            locale_to_kl(Some("hi-in")),
            None,
            "in-hi 不在白名单里就不猜"
        );
        assert_eq!(locale_to_kl(Some("en")), None, "裸语言码不是地区");
        assert_eq!(locale_to_kl(None), None);
    }

    #[test]
    fn query_is_rebuilt_without_the_date_bounds_ddg_ignores() {
        let browser = crate::server::ai::config::BrowserConfig::default();
        let parsed = crate::web::search::query::parse("rust site:tokio.rs -gist before:2024-01-01");
        let ctx = Ctx {
            query: "rust site:tokio.rs -gist before:2024-01-01",
            parsed: &parsed,
            limit: 10,
            recency: Some(super::super::Recency::Week),
            browser: &browser,
        };
        let form: std::collections::HashMap<String, String> =
            build_form(&ctx).into_iter().collect();
        assert!(form["q"].contains("site:tokio.rs"), "{}", form["q"]);
        assert!(form["q"].contains("-gist"), "{}", form["q"]);
        assert!(
            !form["q"].contains("before:"),
            "日期交给后过滤: {}",
            form["q"]
        );
        assert_eq!(form["df"], "w");
        assert_eq!(form["kl"], "us-en");
        assert_eq!(form["b"], "");
    }
}
