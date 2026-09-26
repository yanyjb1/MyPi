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
//! Helium spawns and lives for the process. Calls run on the session's
//! persistent *work tab* — see `session::with_work`.

use anyhow::{Context as _, anyhow};
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;

use super::providers::cmd_act;
use crate::server::ai::config::BrowserConfig;
use crate::web::utils::session::Page;

pub(crate) const ACT_TIMEOUT: Duration = Duration::from_secs(10);
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
        Some(u) => super::super::utils::url::normalize(u)?,
        None => "about:blank".to_string(),
    };
    page.navigate(&target_url)?;
    Ok(format!(
        "browser open: {target_url} (devtools http://127.0.0.1:{}/json)",
        page.port()
    ))
}

fn cmd_read(page: &Page, args: &BrowserArgs) -> anyhow::Result<String> {
    // The page may still be a JS shell when read arrives right after open;
    // poll until the DOM converts to real markdown (best-effort on budget).
    let html = page.wait(super::super::utils::session::RENDER_WAIT, |h| {
        match super::super::fetch::html_to_markdown(h) {
            Ok(md) if super::super::fetch::is_js_shell(h.len(), &md) => None,
            Ok(_) => Some(Ok(())),
            Err(_) => None,
        }
    })?;
    let md = super::super::fetch::html_to_markdown(&html)?;
    if let Some(path) = &args.path {
        std::fs::write(path, &md).with_context(|| format!("writing {path}"))?;
        return Ok(format!("wrote {} chars to {path}", md.chars().count()));
    }
    if md.chars().count() > MAX_OUTPUT_CHARS {
        let cut: String = md.chars().take(MAX_OUTPUT_CHARS).collect();
        Ok(format!(
            "{cut}\n\n[truncated at {MAX_OUTPUT_CHARS} chars — pass path to save the full text]"
        ))
    } else {
        Ok(md)
    }
}

fn cmd_screenshot(page: &Page, args: &BrowserArgs) -> anyhow::Result<String> {
    let path = args
        .path
        .as_deref()
        .ok_or_else(|| anyhow!("screenshot requires path"))?;
    let bytes = page.screenshot(args.full_page.unwrap_or(false))?;
    std::fs::write(path, &bytes).with_context(|| format!("writing {path}"))?;
    Ok(format!("screenshot: {} bytes → {path}", bytes.len()))
}

// --- Tool entry --------------------------------------------------------------

pub fn browser(args: &BrowserArgs, cfg: &BrowserConfig) -> anyhow::Result<String> {
    super::super::utils::session::with_work(cfg, |page| match args.command.as_str() {
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
    use crate::web::utils::url::{base64_decode, normalize};

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
