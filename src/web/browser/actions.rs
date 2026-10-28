use anyhow::{Result, anyhow};
use serde_json::Value;
use std::time::Duration;

use super::page::Page;
use super::session;
use crate::server::config::BrowserConfig;
use crate::web::WebArgs;

pub(crate) const ACTION_TIMEOUT: Duration = Duration::from_secs(10);

fn js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

fn expect(raw: &str, what: &str) -> Result<String> {
    if raw.contains("NOT_FOUND") {
        Err(anyhow!("{what}: selector matched nothing"))
    } else {
        Ok(raw.to_string())
    }
}

fn text(args: &WebArgs, what: &str) -> Result<String> {
    args.value
        .as_ref()
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| anyhow!("{what} requires value (string)"))
}

fn selector(args: &WebArgs, what: &str) -> Result<String> {
    args.selector
        .as_deref()
        .map(js_string)
        .ok_or_else(|| anyhow!("{what} requires selector"))
}

pub(crate) fn run(args: &WebArgs, cfg: &BrowserConfig, session: i64) -> Result<String> {
    let op = args.op.as_str();
    session::with_work(session, cfg, args.reserve, |page: &Page| match op {
        "click" => {
            let sel = selector(args, "click")?;
            let r = page.evaluate(
                &format!("(() => {{ const el = document.querySelector({sel}); if (!el) return 'NOT_FOUND'; el.click(); return 'clicked'; }})()"),
                ACTION_TIMEOUT,
            )?;
            expect(&r, "click")
        }
        "fill" => {
            let sel = selector(args, "fill")?;
            let value = js_string(&text(args, "fill")?);
            let r = page.evaluate(
                &format!("(() => {{ const el = document.querySelector({sel}); if (!el) return 'NOT_FOUND'; el.focus(); el.value = {value}; el.dispatchEvent(new Event('input', {{bubbles: true}})); return 'filled'; }})()"),
                ACTION_TIMEOUT,
            )?;
            expect(&r, "fill")
        }
        "press" => {
            let key = js_string(
                args.selector
                    .as_deref()
                    .ok_or_else(|| anyhow!("press requires selector=key name"))?,
            );
            let r = page.evaluate(
                &format!("(() => {{ const el = document.activeElement; if (!el) return 'NOT_FOUND'; el.dispatchEvent(new KeyboardEvent('keydown', {{key: {key}, bubbles: true}})); el.dispatchEvent(new KeyboardEvent('keyup', {{key: {key}, bubbles: true}})); return 'pressed'; }})()"),
                ACTION_TIMEOUT,
            )?;
            expect(&r, "press")
        }
        "select" => {
            let sel = selector(args, "select")?;
            let value = js_string(&text(args, "select")?);
            let r = page.evaluate(
                &format!("(() => {{ const el = document.querySelector({sel}); if (!el) return 'NOT_FOUND'; el.value = {value}; el.dispatchEvent(new Event('change', {{bubbles: true}})); return 'selected'; }})()"),
                ACTION_TIMEOUT,
            )?;
            expect(&r, "select")
        }
        "scroll" => {
            let dy = args.value.as_ref().and_then(Value::as_i64).unwrap_or(600);
            let r = page.evaluate(
                &format!("window.scrollBy(0, {dy}); 'scrolled'"),
                ACTION_TIMEOUT,
            )?;
            expect(&r, "scroll")
        }
        "eval" => {
            let expr = text(args, "eval")?;
            page.evaluate(&expr, ACTION_TIMEOUT)
        }
        other => Err(anyhow!(
            "unknown op {other:?}; click|fill|press|select|scroll|eval"
        )),
    })
}
