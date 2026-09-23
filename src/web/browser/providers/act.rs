//! Interactive `act` operations: navigate/click/fill/press/select/scroll/
//! eval — one match arm per op over the session's `Page`.

use anyhow::anyhow;
use serde_json::Value;
use super::super::engine::BrowserArgs;
use crate::web::utils::session::Page;

pub(crate) fn cmd_act(page: &Page, args: &BrowserArgs) -> anyhow::Result<String> {
    let op = args.op.as_deref().ok_or_else(|| anyhow!("act requires op"))?;
    let js_escape = |s: &str| s.replace('\\', "\\\\").replace('\'', "\\'");

    match op {
        "navigate" => {
            let url = args.url.as_deref().ok_or_else(|| anyhow!("navigate requires url"))?;
            let target = crate::web::utils::url::normalize(url)?;
            page.navigate(&target)?;
            Ok(format!("navigated: {target}"))
        }
        "click" => {
            let sel = js_escape(args.selector.as_deref().ok_or_else(|| anyhow!("click requires selector"))?);
            let r = page.evaluate(
                &format!("(() => {{ const el = document.querySelector('{sel}'); if (!el) return 'NOT_FOUND'; el.click(); return 'clicked'; }})()"),
                super::super::engine::ACT_TIMEOUT,
            )?;
            Ok(expect(&r, "click")?)
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
                super::super::engine::ACT_TIMEOUT,
            )?;
            Ok(expect(&r, "fill")?)
        }
        "press" => {
            let key = args.selector.as_deref().ok_or_else(|| anyhow!("press requires selector=key name"))?;
            let r = page.evaluate(
                &format!("(() => {{ const el = document.activeElement; if (!el) return 'NOT_FOUND'; el.dispatchEvent(new KeyboardEvent('keydown', {{key: '{key}', bubbles: true}})); el.dispatchEvent(new KeyboardEvent('keyup', {{key: '{key}', bubbles: true}})); return 'pressed'; }})()"),
                super::super::engine::ACT_TIMEOUT,
            )?;
            Ok(expect(&r, "press")?)
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
                super::super::engine::ACT_TIMEOUT,
            )?;
            Ok(expect(&r, "select")?)
        }
        "scroll" => {
            let dy = args.value.as_ref().and_then(Value::as_i64).unwrap_or(600);
            let r = page.evaluate(&format!("window.scrollBy(0, {dy}); 'scrolled'"), super::super::engine::ACT_TIMEOUT)?;
            Ok(expect(&r, "scroll")?)
        }
        "eval" => {
            let expr = args
                .value
                .as_ref()
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("eval requires value (js expression)"))?;
            page.evaluate(expr, super::super::engine::ACT_TIMEOUT)
        }
        "net" => super::net::capture_network(page, args),
        other => Err(anyhow!(
            "unknown op {other:?}; navigate|click|fill|press|select|scroll|eval|net"
        )),
    }
}

pub(crate) fn expect(raw: &str, what: &str) -> anyhow::Result<String> {
    if raw.contains("NOT_FOUND") {
        Err(anyhow!("{what}: selector matched nothing"))
    } else {
        Ok(raw.to_string())
    }
}
