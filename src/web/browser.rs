//! Browser tool — four verbs over the CDP layer: `open`, `act`, `read`,
//! `screenshot`.
//!
//! Schema keeps every parameter explainable; there is no eval-runtime dialect
//! to learn (the omp lesson: 60+ methods behind a code-string action made the
//! schema unspeakable). Interaction primitives live on `act.op`:
//! navigate/click/fill/press/select/scroll/eval — plus `net` for request
//! capture, which is what "find the API this page calls" needs.
//!
//! Browser source: `MYPI_BROWSER_PORT` attaches to a long-lived instance
//! (session warmth matters to anti-bot frontends); otherwise a headless
//! Helium spawns on demand and is reaped when the call ends.

use anyhow::{Context as _, anyhow};
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;

use super::session::Page;


const ACT_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_OUTPUT_CHARS: usize = 24_000;

#[derive(Debug, Deserialize)]
pub struct BrowserArgs {
    /// 一句话说明这次调用要干什么，中文，会显示给用户看
    pub intent: String,
    /// open | act | read | screenshot
    pub command: String,
    /// open: 要导航到的 URL（省略则打开空白页）
    pub url: Option<String>,
    /// act 的操作: navigate|click|fill|press|select|scroll|eval|net
    pub op: Option<String>,
    /// act 的目标：CSS 选择器（click/fill/select/scroll）或按键名（press）
    pub selector: Option<String>,
    /// fill 的文本 / eval 的 JS 表达式 / scroll 的像素
    pub value: Option<Value>,
    /// net: 只保留 URL 含此子串的请求
    pub filter: Option<String>,
    /// net: 最多返回多少条请求（默认 20）
    pub max: Option<usize>,
    /// read/screenshot: 输出文件路径（screenshot 必填；read 省略则直接返回文本）
    pub path: Option<String>,
    /// screenshot: 整页截图（默认视口）
    pub full_page: Option<bool>,
}

pub fn parse_browser_args(arguments: &str) -> anyhow::Result<BrowserArgs> {
    serde_json::from_str(arguments).map_err(|e| anyhow!("bad browser args: {e}"))
}

// --- Command implementations -------------------------------------------------

fn cmd_open(page: &Page, url: Option<&str>) -> anyhow::Result<String> {
    let target_url = match url {
        Some(u) => super::url::normalize(u)?,
        None => "about:blank".to_string(),
    };
    page.navigate(&target_url)?;
    Ok(format!(
        "browser open: {target_url} (devtools http://127.0.0.1:{}/json)",
        page.port()
    ))
}


fn cmd_act(page: &Page, args: &BrowserArgs) -> anyhow::Result<String> {
    let op = args.op.as_deref().ok_or_else(|| anyhow!("act requires op"))?;
    let js_escape = |s: &str| s.replace('\\', "\\\\").replace('\'', "\\'");

    match op {
        "navigate" => {
            let url = args.url.as_deref().ok_or_else(|| anyhow!("navigate requires url"))?;
            let target = super::url::normalize(url)?;
            page.navigate(&target)?;
            Ok(format!("navigated: {target}"))
        }
        "click" => {
            let sel = js_escape(args.selector.as_deref().ok_or_else(|| anyhow!("click requires selector"))?);
            let r = page.evaluate(
                &format!("(() => {{ const el = document.querySelector('{sel}'); if (!el) return 'NOT_FOUND'; el.click(); return 'clicked'; }})()"),
                ACT_TIMEOUT,
            )?;
            Ok(expect(&r, "clicked", "click")?)
        }
        "fill" => {
            let sel = js_escape(args.selector.as_deref().ok_or_else(|| anyhow!("fill requires selector"))?);
            let text = args
                .value
                .as_ref()
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("fill requires value (string)"))?;
            let escaped = js_escape(text);
            let r = page.evaluate(
                &format!("(() => {{ const el = document.querySelector('{sel}'); if (!el) return 'NOT_FOUND'; el.focus(); el.value = '{escaped}'; el.dispatchEvent(new Event('input', {{bubbles: true}})); return 'filled'; }})()"),
                ACT_TIMEOUT,
            )?;
            Ok(expect(&r, "filled", "fill")?)
        }
        "press" => {
            let key = args.selector.as_deref().ok_or_else(|| anyhow!("press requires selector=key name"))?;
            let r = page.evaluate(
                &format!("(() => {{ const el = document.activeElement; if (!el) return 'NOT_FOUND'; el.dispatchEvent(new KeyboardEvent('keydown', {{key: '{key}', bubbles: true}})); el.dispatchEvent(new KeyboardEvent('keyup', {{key: '{key}', bubbles: true}})); return 'pressed'; }})()"),
                ACT_TIMEOUT,
            )?;
            Ok(expect(&r, "pressed", "press")?)
        }
        "select" => {
            let sel = js_escape(args.selector.as_deref().ok_or_else(|| anyhow!("select requires selector"))?);
            let text = args
                .value
                .as_ref()
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("select requires value (option value)"))?;
            let escaped = js_escape(text);
            let r = page.evaluate(
                &format!("(() => {{ const el = document.querySelector('{sel}'); if (!el) return 'NOT_FOUND'; el.value = '{escaped}'; el.dispatchEvent(new Event('change', {{bubbles: true}})); return 'selected'; }})()"),
                ACT_TIMEOUT,
            )?;
            Ok(expect(&r, "selected", "select")?)
        }
        "scroll" => {
            let dy = args.value.as_ref().and_then(Value::as_i64).unwrap_or(600);
            let r = page.evaluate(&format!("window.scrollBy(0, {dy}); 'scrolled'"), ACT_TIMEOUT)?;
            Ok(expect(&r, "scrolled", "scroll")?)
        }
        "eval" => {
            let expr = args
                .value
                .as_ref()
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("eval requires value (js expression)"))?;
            page.evaluate(expr, ACT_TIMEOUT)
        }
        "net" => capture_network(page, args),
        other => Err(anyhow!(
            "unknown op {other:?}; navigate|click|fill|press|select|scroll|eval|net"
        )),
    }
}

fn expect(raw: &str, _want: &str, what: &str) -> anyhow::Result<String> {
    if raw.contains("NOT_FOUND") {
        Err(anyhow!("{what}: selector matched nothing"))
    } else {
        Ok(raw.to_string())
    }
}

// --- Network capture ---------------------------------------------------------
//
// "Find the API this page calls": Network.enable before navigation, then
// collect requestWillBeSent events. The pump already forwards every frame
// into the session inbox; Network events are interleaved there, so capture
// works by enabling the domain, re-navigating, and draining.

// Events arrive through the Cdp inbox mixed with responses; rather than
// reaching into Cdp's internals, we spawn a fresh CDP connection dedicated
// to the recorder — a second WS per target is supported by Chromium.
fn capture_network(page: &Page, args: &BrowserArgs) -> anyhow::Result<String> {
    let filter = args.filter.clone().unwrap_or_default();
    let max = args.max.unwrap_or(20).clamp(1, 100);

    // Ask the page for its own performance entries: zero protocol surface,
    // no event races, and it covers every request the page actually made
    // (XHR/fetch/script/img) with initiatorType labels.
    let expr = format!(
        "JSON.stringify((performance.getEntriesByType('resource')||[]).slice(-{max}).map(e => ({{name: e.name, type: e.initiatorType, ms: Math.round(e.duration)}})))"
    );
    let raw = page.evaluate(&expr, ACT_TIMEOUT)?;
    let entries: Vec<Value> = serde_json::from_str(&raw).unwrap_or_default();

    let mut out = String::from("network requests (newest last):\n");
    let mut shown = 0;
    for e in &entries {
        let name = e.get("name").and_then(Value::as_str).unwrap_or("");
        if !filter.is_empty() && !name.contains(&filter) {
            continue;
        }
        shown += 1;
        if shown > max {
            break;
        }
        out.push_str(&format!(
            "  [{}] {} ({})\n",
            e.get("type").and_then(Value::as_str).unwrap_or("?"),
            name,
            e.get("ms").and_then(Value::as_i64).unwrap_or(0)
        ));
    }
    if shown == 0 {
        out.push_str(&format!("  (no requests matching {filter:?})\n"));
    }
    Ok(out)
}

// --- read / screenshot -------------------------------------------------------

fn cmd_read(page: &Page, args: &BrowserArgs) -> anyhow::Result<String> {
    let html = page.dom_html()?;
    let md = super::fetch::html_to_markdown(&html)?;
    if let Some(path) = &args.path {
        std::fs::write(path, &md).with_context(|| format!("writing {path}"))?;
        return Ok(format!("wrote {} chars to {path}", md.chars().count()));
    }
    if md.chars().count() > MAX_OUTPUT_CHARS {
        let cut: String = md.chars().take(MAX_OUTPUT_CHARS).collect();
        Ok(format!("{cut}\n\n[truncated at {MAX_OUTPUT_CHARS} chars — pass path to save the full text]"))
    } else {
        Ok(md)
    }
}

fn cmd_screenshot(page: &Page, args: &BrowserArgs) -> anyhow::Result<String> {
    let path = args.path.as_deref().ok_or_else(|| anyhow!("screenshot requires path"))?;
    let bytes = page.screenshot(args.full_page.unwrap_or(false))?;
    std::fs::write(path, &bytes).with_context(|| format!("writing {path}"))?;
    Ok(format!("screenshot: {} bytes → {path}", bytes.len()))
}

// --- Tool entry --------------------------------------------------------------

pub fn browser(args: &BrowserArgs) -> anyhow::Result<String> {
    super::session::with_page(|page| match args.command.as_str() {
        "open" => cmd_open(page, args.url.as_deref()),
        "act" => cmd_act(page, args),
        "read" => cmd_read(page, args),
        "screenshot" => cmd_screenshot(page, args),
        other => Err(anyhow!(
            "unknown command {other:?}; open|act|read|screenshot"
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::super::url::{base64_decode, normalize};

    #[test]
    fn base64_decode_roundtrips_png_magic() {
        // b64 of bytes [0x89, 0x50, 0x4E, 0x47] (PNG magic)
        let enc = "iVBORw0KGgo=";
        let d = base64_decode(enc).unwrap();
        assert_eq!(&d[..4], &[0x89, 0x50, 0x4E, 0x47]);
    }

    #[test]
    fn base64_decode_rejects_garbage() {
        assert!(base64_decode("!!!!").is_err());
    }

    #[test]
    fn normalize_blocks_local_schemes() {
        assert!(normalize("file:///etc/passwd").is_err());
        assert!(normalize("javascript:alert(1)").is_err());
        assert_eq!(normalize("example.com").unwrap(), "https://example.com");
        assert_eq!(normalize("http://x.y/").unwrap(), "http://x.y/");
    }
}
