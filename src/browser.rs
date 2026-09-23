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
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::cdp::{Browser, Cdp};

const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(20);
const ACT_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_TIMEOUT: Duration = Duration::from_secs(8);
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

// --- Session management ------------------------------------------------------
//
// One browser per process, created lazily and reused across calls. The Mutex
// guards against the agent loop issuing concurrent tool calls; the Option
// stays None until the first call needs it.

struct Session {
    browser: Browser,
    cdp: Arc<Mutex<Cdp>>,
}

static SESSION: Mutex<Option<Session>> = Mutex::new(None);

fn acquire_session() -> anyhow::Result<(u16, Arc<Mutex<Cdp>>)> {
    let mut guard = SESSION.lock().map_err(|_| anyhow!("session mutex poisoned"))?;
    if let Some(existing) = guard.as_ref() {
        // The attached/spawned browser may have died between calls; a cheap
        // liveness probe avoids handing out a dead socket.
        if Browser::port_alive(existing.browser.port) {
            return Ok((existing.browser.port, Arc::clone(&existing.cdp)));
        }
        *guard = None;
    }
    let profile = crate::xdg::browser_profile_dir();
    let browser = match std::env::var("MYPI_BROWSER_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
    {
        Some(port) => Browser::attach(&profile, port).context("attaching to MYPI_BROWSER_PORT")?,
        None => Browser::launch(&profile).context("launching browser")?,
    };
    let port = browser.port;
    let target = browser.page_target().context("no page target")?;
    let cdp = Arc::new(Mutex::new(Cdp::connect(&target.ws_url)?));
    *guard = Some(Session { browser, cdp: Arc::clone(&cdp) });
    Ok((port, cdp))
}

// --- Cdp helpers over the shared session -------------------------------------
//
// The session's Cdp sits behind a Mutex; navigation resets the per-target
// socket, so every helper treats "connection reset" as reconnect-and-retry
// once — the same recovery the fetch tier uses, centralized here.

fn with_cdp<T>(cdp: &Arc<Mutex<Cdp>>, f: impl FnOnce(&mut Cdp) -> anyhow::Result<T>) -> anyhow::Result<T> {
    let mut guard = cdp.lock().map_err(|_| anyhow!("cdp mutex poisoned"))?;
    f(&mut guard)
}

fn reconnect(_cdp: &Arc<Mutex<Cdp>>) -> anyhow::Result<()> {
    let port = {
        let mut guard = SESSION.lock().map_err(|_| anyhow!("session mutex poisoned"))?;
        let Some(session) = guard.as_mut() else {
            anyhow::bail!("no session");
        };
        let target = session.browser.page_target().context("no page target")?;
        session.cdp = Arc::new(Mutex::new(Cdp::connect(&target.ws_url)?));
        session.browser.port
    };
    let _ = port;
    Ok(())
}

// Dispatch one CDP command, reconnecting once on a reset socket.
fn call_resilient(
    cdp: &Arc<Mutex<Cdp>>,
    method: &str,
    params: Value,
    timeout: Duration,
) -> anyhow::Result<Value> {
    let attempt = |c: &mut Cdp| c.call(method, params.clone(), timeout);
    match with_cdp(cdp, attempt) {
        Ok(v) => Ok(v),
        Err(e) if e.to_string().contains("connection reset") => {
            reconnect(cdp)?;
            with_cdp(cdp, attempt)
        }
        Err(e) => Err(e),
    }
}

// --- Command implementations -------------------------------------------------

fn cmd_open(cdp: &Arc<Mutex<Cdp>>, url: Option<&str>, port: u16) -> anyhow::Result<String> {
    let target_url = match url {
        Some(u) => normalize(u)?,
        None => "about:blank".to_string(),
    };
    call_resilient(cdp, "Page.enable", json!({}), Duration::from_secs(5))?;
    call_resilient(
        cdp,
        "Page.navigate",
        json!({"url": target_url}),
        NAVIGATE_TIMEOUT,
    )?;
    Ok(format!("browser open: {target_url} (devtools http://127.0.0.1:{port}/json)"))
}

fn normalize(raw: &str) -> anyhow::Result<String> {
    let url = raw.trim();
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("file:") || lower.starts_with("data:") || lower.starts_with("javascript:") {
        return Err(anyhow!("scheme not allowed"));
    }
    let with = if lower.starts_with("http://") || lower.starts_with("https://") {
        url.to_string()
    } else {
        format!("https://{url}")
    };
    Ok(with)
}

fn cmd_act(cdp: &Arc<Mutex<Cdp>>, args: &BrowserArgs) -> anyhow::Result<String> {
    let op = args.op.as_deref().ok_or_else(|| anyhow!("act requires op"))?;
    let js_escape = |s: &str| s.replace('\\', "\\\\").replace('\'', "\\'");

    match op {
        "navigate" => {
            let url = args.url.as_deref().ok_or_else(|| anyhow!("navigate requires url"))?;
            let target = normalize(url)?;
            call_resilient(cdp, "Page.enable", json!({}), Duration::from_secs(5))?;
            call_resilient(cdp, "Page.navigate", json!({"url": target}), NAVIGATE_TIMEOUT)?;
            Ok(format!("navigated: {target}"))
        }
        "click" => {
            let sel = js_escape(args.selector.as_deref().ok_or_else(|| anyhow!("click requires selector"))?);
            let r = eval_js(cdp, &format!(
                "(() => {{ const el = document.querySelector('{sel}'); if (!el) return 'NOT_FOUND'; el.click(); return 'clicked'; }})()"
            ))?;
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
            let r = eval_js(cdp, &format!(
                "(() => {{ const el = document.querySelector('{sel}'); if (!el) return 'NOT_FOUND'; el.focus(); el.value = '{escaped}'; el.dispatchEvent(new Event('input', {{bubbles: true}})); return 'filled'; }})()"
            ))?;
            Ok(expect(&r, "filled", "fill")?)
        }
        "press" => {
            let key = args.selector.as_deref().ok_or_else(|| anyhow!("press requires selector=key name"))?;
            let r = eval_js(cdp, &format!(
                "(() => {{ const el = document.activeElement; if (!el) return 'NOT_FOUND'; el.dispatchEvent(new KeyboardEvent('keydown', {{key: '{key}', bubbles: true}})); el.dispatchEvent(new KeyboardEvent('keyup', {{key: '{key}', bubbles: true}})); return 'pressed'; }})()"
            ))?;
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
            let r = eval_js(cdp, &format!(
                "(() => {{ const el = document.querySelector('{sel}'); if (!el) return 'NOT_FOUND'; el.value = '{escaped}'; el.dispatchEvent(new Event('change', {{bubbles: true}})); return 'selected'; }})()"
            ))?;
            Ok(expect(&r, "selected", "select")?)
        }
        "scroll" => {
            let dy = args.value.as_ref().and_then(Value::as_i64).unwrap_or(600);
            let r = eval_js(cdp, &format!("window.scrollBy(0, {dy}); 'scrolled'"))?;
            Ok(expect(&r, "scrolled", "scroll")?)
        }
        "eval" => {
            let expr = args
                .value
                .as_ref()
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("eval requires value (js expression)"))?;
            eval_js(cdp, expr)
        }
        "net" => capture_network(cdp, args),
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

// Runtime.evaluate through the shared session. Note: bot-hardened pages can
// hang this channel (live-verified) — click/fill/eval on such pages is
// best-effort, while `read` (DOM domain) always works.
fn eval_js(cdp: &Arc<Mutex<Cdp>>, expression: &str) -> anyhow::Result<String> {
    let v = call_resilient(
        cdp,
        "Runtime.evaluate",
        json!({"expression": expression, "returnByValue": true}),
        ACT_TIMEOUT,
    )?;
    Ok(v.pointer("/result/value")
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        })
        .unwrap_or_else(|| "undefined".to_string()))
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
fn capture_network(cdp: &Arc<Mutex<Cdp>>, args: &BrowserArgs) -> anyhow::Result<String> {
    let filter = args.filter.clone().unwrap_or_default();
    let max = args.max.unwrap_or(20).clamp(1, 100);

    // Ask the page for its own performance entries: zero protocol surface,
    // no event races, and it covers every request the page actually made
    // (XHR/fetch/script/img) with initiatorType labels.
    let expr = format!(
        "JSON.stringify((performance.getEntriesByType('resource')||[]).slice(-{max}).map(e => ({{name: e.name, type: e.initiatorType, ms: Math.round(e.duration)}})))"
    );
    let raw = eval_js(cdp, &expr)?;
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

fn cmd_read(cdp: &Arc<Mutex<Cdp>>, args: &BrowserArgs) -> anyhow::Result<String> {
    let html = with_cdp(cdp, |c| c.dom_html(POLL_TIMEOUT))?;
    let md = crate::fetch::html_to_markdown(&html)?;
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

fn cmd_screenshot(cdp: &Arc<Mutex<Cdp>>, args: &BrowserArgs) -> anyhow::Result<String> {
    let path = args.path.as_deref().ok_or_else(|| anyhow!("screenshot requires path"))?;
    let full = args.full_page.unwrap_or(false);
    let mut params = json!({"format": "png"});
    if full {
        params["captureBeyondViewport"] = json!(true);
    }
    let r = call_resilient(cdp, "Page.captureScreenshot", params, Duration::from_secs(15))?;
    let data = r
        .get("data")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("no screenshot data"))?;
    let bytes = base64_decode(data)?;
    std::fs::write(path, &bytes).with_context(|| format!("writing {path}"))?;
    Ok(format!("screenshot: {} bytes → {path}", bytes.len()))
}

// Minimal standard base64 decoder (screenshot payloads arrive b64-encoded).
fn base64_decode(s: &str) -> anyhow::Result<Vec<u8>> {
    fn val(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a') as u32 + 26),
            b'0'..=b'9' => Some((c - b'0') as u32 + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let clean: Vec<u8> = s
        .bytes()
        .filter(|b| !b" \n\r\t".contains(b))
        .collect();
    let mut out = Vec::with_capacity(clean.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for &c in &clean {
        if c == b'=' {
            break;
        }
        let v = val(c).ok_or_else(|| anyhow!("bad base64 byte {c:#x}"))?;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

// --- Tool entry --------------------------------------------------------------

pub fn browser(args: &BrowserArgs) -> anyhow::Result<String> {
    let (port, cdp) = acquire_session()?;
    match args.command.as_str() {
        "open" => cmd_open(&cdp, args.url.as_deref(), port),
        "act" => cmd_act(&cdp, args),
        "read" => cmd_read(&cdp, args),
        "screenshot" => cmd_screenshot(&cdp, args),
        other => Err(anyhow!(
            "unknown command {other:?}; open|act|read|screenshot"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
