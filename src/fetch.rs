//! Web fetch — read a URL as clean markdown for the model's context.
//!
//! Tier 1: plain HTTPS GET (ureq) with a browser-shaped UA, then
//! `html-to-markdown-rs` with the Aggressive preprocessing preset (drops
//! nav/footer/forms/ads markup before conversion).
//!
//! Tier 2: when the direct GET yields a JS-shell page (tiny body, no text)
//! or the site blocks non-browser traffic, re-fetch through headless Helium
//! and pull the **rendered** DOM via the DOM domain — the read path that
//! survives pages whose `Runtime.evaluate` channel anti-bot JS hangs (see
//! `cdp::dom_html`). The same markdown conversion runs on the rendered DOM.
//!
//! Output budget: hard-capped at `MAX_OUTPUT_CHARS` with a note when cut —
//! one `document.body.innerText` must not eat the context window.

use anyhow::{Context as _, anyhow};
use html_to_markdown_rs::convert;
use html_to_markdown_rs::options::{ConversionOptions, PreprocessingOptions, PreprocessingPreset};
use serde::Deserialize;
use std::time::{Duration, Instant};

const USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/153.0.0.0 Safari/537.36";
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);
const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(20);
const RENDER_WAIT: Duration = Duration::from_secs(12);
const POLL_TIMEOUT: Duration = Duration::from_secs(8);

// Context-window guard. ~4 pages of text; omp caps at 500k which is far
// beyond what a model can use in one tool result.
const MAX_OUTPUT_CHARS: usize = 24_000;

#[derive(Debug, Deserialize)]
pub struct FetchArgs {
    /// 一句话说明这次调用要干什么，中文，会显示给用户看
    pub intent: String,
    /// 要读取的 URL（http/https）
    pub url: String,
    /// 返回原始 HTML 而不是 markdown（默认 false）
    pub raw: Option<bool>,
}

pub fn parse_fetch_args(arguments: &str) -> anyhow::Result<FetchArgs> {
    serde_json::from_str(arguments).map_err(|e| anyhow!("bad fetch args: {e}"))
}

// Scheme guard: only http(s). file:// would read local disks, and every
// other scheme (ftp:, data:, chrome:) is either unwanted or a hazard.
fn normalize_url(raw: &str) -> anyhow::Result<String> {
    let url = raw.trim();
    // Guard BEFORE prefixing: "file:///etc/passwd" has no http prefix, and
    // blind prepending would mint "https://file:///etc/passwd" whose host
    // parses as "file" — a live-looking URL that actually fetches nothing.
    // Reject known-local schemes outright, then prefix the bare-domain form.
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("file:")
        || lower.starts_with("data:")
        || lower.starts_with("ftp:")
        || lower.starts_with("chrome:")
        || lower.starts_with("javascript:")
    {
        return Err(anyhow!("scheme not allowed (http/https only)"));
    }
    let with_scheme = if lower.starts_with("http://") || lower.starts_with("https://") {
        url.to_string()
    } else {
        format!("https://{url}")
    };
    let parsed = url::Url::parse(&with_scheme).map_err(|e| anyhow!("bad url: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => Ok(with_scheme),
        other => Err(anyhow!("scheme {other:?} not allowed (http/https only)")),
    }
}

// --- Markdown conversion -----------------------------------------------------

/// HTML → markdown with the aggressive cleanup preset. Public for tests.
pub fn html_to_markdown(html: &str) -> anyhow::Result<String> {
    let options = ConversionOptions {
        preprocessing: PreprocessingOptions {
            enabled: true,
            preset: PreprocessingPreset::Aggressive,
            ..Default::default()
        },
        ..Default::default()
    };
    let result = convert(html, Some(options)).context("html-to-markdown conversion")?;
    Ok(result.content.unwrap_or_default())
}

fn truncate(content: &str) -> (String, bool) {
    if content.chars().count() <= MAX_OUTPUT_CHARS {
        return (content.to_string(), false);
    }
    let cut: String = content.chars().take(MAX_OUTPUT_CHARS).collect();
    (cut, true)
}

// --- Tier 1: direct ----------------------------------------------------------

// Live-verified failure shapes worth falling back on:
//  • non-2xx from bot gates (403/503 + cloudflare/captcha markers)
//  • 200 but the body is a JS shell — the extraction yields almost no text
//    relative to the raw size (say, under 200 chars out of 20 KB)
fn looks_bot_blocked(status: u16, body: &str) -> bool {
    if status == 403 || status == 503 || status == 429 {
        let lower = body.to_lowercase();
        return lower.contains("cloudflare")
            || lower.contains("captcha")
            || lower.contains("challenge")
            || lower.contains("access denied");
    }
    false
}

fn is_js_shell(raw_len: usize, converted: &str) -> bool {
    // Two shells the first heuristic missed live:
    //  1. tiny text out of a big body (classic React root div)
    //  2. a big body whose markdown is mostly `meta-*` dump lines with no
    //     real paragraphs — YouTube serves megabytes of bootstrap JS whose
    //     only conversion output is metadata rows
    if converted.trim().len() < 200 && raw_len > 10_000 {
        return true;
    }
    let meta_lines = converted
        .lines()
        .filter(|l| l.starts_with("meta-") || l.starts_with("title:"))
        .count();
    let body_lines = converted
        .lines()
        .filter(|l| !l.starts_with("meta-") && !l.starts_with("title:") && !l.trim().is_empty())
        .count();
    meta_lines >= 10 && body_lines < 15
}

fn fetch_direct(url: &str) -> anyhow::Result<(String, u16, usize)> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        .build()
        .into();
    let mut response = agent
        .get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "text/html,application/xhtml+xml")
        .header("Accept-Language", "en-US,en;q=0.9")
        .call()
        .map_err(|e| anyhow!("request failed: {e}"))?;
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().context("reading body")?;
    let len = body.len();
    Ok((body, status, len))
}

// --- Tier 2: browser ---------------------------------------------------------

fn fetch_via_browser(url: &str) -> anyhow::Result<String> {
    let profile = crate::xdg::browser_profile_dir();
    let browser = match std::env::var("MYPI_BROWSER_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
    {
        Some(port) => crate::cdp::Browser::attach(&profile, port)
            .context("attaching to MYPI_BROWSER_PORT")?,
        None => crate::cdp::Browser::launch(&profile).context("launching browser")?,
    };
    let mut target = browser.page_target().context("no page target")?;
    let mut cdp = crate::cdp::Cdp::connect(&target.ws_url)?;

    cdp.navigate(url, NAVIGATE_TIMEOUT)?;

    let deadline = Instant::now() + RENDER_WAIT;
    loop {
        let html = match cdp.dom_html(POLL_TIMEOUT) {
            Ok(html) => html,
            Err(e) if e.to_string().contains("connection reset") => {
                // Navigation reset the socket; re-resolve (tab id survives)
                // and keep polling.
                target = browser.page_target().context("no page target")?;
                cdp = crate::cdp::Cdp::connect(&target.ws_url)?;
                continue;
            }
            Err(e) => return Err(e),
        };
        // Readiness: the DOM has real content once innerText-style length
        // shows up. A cheap proxy — the DOM tree carries more than the bare
        // shell script tags.
        if html.len() > 2_000 {
            return Ok(html);
        }
        if Instant::now() > deadline {
            return Ok(html); // render what we have rather than fail
        }
        std::thread::sleep(Duration::from_millis(400));
    }
}

/// Fetch a URL and return model-facing markdown. Public entry for the tool
/// layer and tests.
pub fn fetch(args: &FetchArgs) -> anyhow::Result<String> {
    let url = normalize_url(&args.url)?;
    let raw = args.raw.unwrap_or(false);

    enum Tier {
        /// Direct GET produced model-ready content.
        Direct(String),
        /// Raw mode skips conversion entirely (works direct or browser).
        Raw(String),
        /// Direct failed / bot-walled / JS shell — try the browser.
        Fallback,
    }

    // Tier 1 — direct GET.
    let tier = match fetch_direct(&url) {
        Ok((body, status, len)) if !looks_bot_blocked(status, &body) => {
            if raw {
                Tier::Raw(body)
            } else {
                match html_to_markdown(&body) {
                    Ok(md) if !is_js_shell(len, &md) => Tier::Direct(md),
                    // Conversion yields a JS shell: browser tier may render
                    // real content. Conversion errors also fall through —
                    // the browser gets one clean shot at the page.
                    _ => Tier::Fallback,
                }
            }
        }
        _ => Tier::Fallback,
    };

    // Tier 2 — browser fallback, only when tier 1 couldn't serve.
    match tier {
        Tier::Direct(md) => {
            let (out, cut) = truncate(&md);
            Ok(format_output(&url, "direct", &out, cut))
        }
        Tier::Raw(body) => {
            let (out, cut) = truncate(&body);
            Ok(format_output(&url, "raw", &out, cut))
        }
        Tier::Fallback => {
            let html = fetch_via_browser(&url)?;
            let converted = if raw { html } else { html_to_markdown(&html)? };
            let (out, cut) = truncate(&converted);
            Ok(format_output(&url, "browser", &out, cut))
        }
    }
}


fn format_output(url: &str, method: &str, content: &str, truncated: bool) -> String {
    let mut out = format!("URL: {url}\nMethod: {method}\n");
    if truncated {
        out.push_str(&format!(
            "Note: content truncated at {MAX_OUTPUT_CHARS} chars; use a more specific tool or URL for the rest\n"
        ));
    }
    out.push_str("\n---\n\n");
    out.push_str(content);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_normalization_and_scheme_guard() {
        assert_eq!(
            normalize_url("example.com/x").unwrap(),
            "https://example.com/x"
        );
        assert_eq!(
            normalize_url("http://a.b/").unwrap(),
            "http://a.b/"
        );
        assert!(normalize_url("file:///etc/passwd").is_err());
        assert!(normalize_url("ftp://x/").is_err());
        assert!(normalize_url("data:text/html,x").is_err());
        assert!(normalize_url("").is_err());
    }

    #[test]
    fn markdown_conversion_strips_chrome_and_keeps_text() {
        let html = r#"<html><head><title>T</title></head><body>
            <nav>menu item 1 | menu item 2</nav>
            <h1>Real Heading</h1>
            <p>Real paragraph with <b>bold</b> text.</p>
            <form><input name="q"><button>Go</button></form>
            <footer>copyright footer junk</footer>
        </body></html>"#;
        let md = html_to_markdown(html).unwrap();
        assert!(md.contains("Real Heading"), "{md}");
        assert!(md.contains("Real paragraph"), "{md}");
        // Aggressive preprocessing removes form/nav/footer chrome.
        assert!(!md.contains("menu item"), "{md}");
        assert!(!md.contains("copyright footer"), "{md}");
    }

    #[test]
    fn js_shell_detection_flags_tiny_text_from_big_body() {
        assert!(is_js_shell(20_000, "  # App\nLoading…"));
        assert!(!is_js_shell(5_000, "short page"));
        let real = "x".repeat(5_000);
        assert!(!is_js_shell(20_000, &real));
    }

    #[test]
    fn bot_block_markers_match_live_gates() {
        assert!(looks_bot_blocked(403, "<html>Attention Required! | Cloudflare</html>"));
        assert!(looks_bot_blocked(503, "captcha challenge"));
        assert!(!looks_bot_blocked(200, "<html>normal</html>"));
        assert!(!looks_bot_blocked(403, "plain forbidden text without markers"));
    }

    #[test]
    fn output_truncation_kicks_in_at_the_cap() {
        let big = "x".repeat(MAX_OUTPUT_CHARS + 100);
        let (out, cut) = truncate(&big);
        assert!(cut);
        assert_eq!(out.chars().count(), MAX_OUTPUT_CHARS);
        let small = "tiny";
        assert!(!truncate(small).1);
    }

    #[test]
    fn output_header_carries_url_method_and_truncation_note() {
        let out = format_output("https://x/", "direct", "body", true);
        assert!(out.starts_with("URL: https://x/\nMethod: direct\n"));
        assert!(out.contains("truncated"));
        let out2 = format_output("https://x/", "direct", "body", false);
        assert!(!out2.contains("truncated"));
    }
}
