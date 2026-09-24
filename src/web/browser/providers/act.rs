//! Interactive `act` operations: navigate/click/fill/press/select/scroll/
//! eval — one match arm per op over the session's `Page`.

use super::super::engine::BrowserArgs;
use crate::web::utils::session::Page;
use anyhow::anyhow;
use serde_json::Value;

/// A single-quoted JavaScript string literal holding `s` verbatim.
///
/// The old helper escaped only `\` and `'`, so any value containing a newline
/// (a multi-line `fill`, say) produced a syntax error and the whole `act` call
/// failed with a JS parse message instead of doing the work. Control
/// characters and the JS line separators are escaped too — they terminate a
/// literal just like a real newline does.
pub(crate) fn js_string(s: &str) -> String {
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

pub(crate) fn cmd_act(page: &Page, args: &BrowserArgs) -> anyhow::Result<String> {
    let op = args
        .op
        .as_deref()
        .ok_or_else(|| anyhow!("act requires op"))?;
    let selector_of = |what: &str| -> anyhow::Result<String> {
        args.selector
            .as_deref()
            .map(js_string)
            .ok_or_else(|| anyhow!("{what} requires selector"))
    };

    match op {
        "navigate" => {
            let url = args
                .url
                .as_deref()
                .ok_or_else(|| anyhow!("navigate requires url"))?;
            let target = crate::web::utils::url::normalize(url)?;
            page.navigate(&target)?;
            Ok(format!("navigated: {target}"))
        }
        "click" => {
            let sel = selector_of("click")?;
            let r = page.evaluate(
                &format!("(() => {{ const el = document.querySelector({sel}); if (!el) return 'NOT_FOUND'; el.click(); return 'clicked'; }})()"),
                super::super::engine::ACT_TIMEOUT,
            )?;
            Ok(expect(&r, "click")?)
        }
        "fill" => {
            let sel = selector_of("fill")?;
            let text = args
                .value
                .as_ref()
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("fill requires value (string)"))?;
            let escaped = js_string(text);
            let r = page.evaluate(
                &format!("(() => {{ const el = document.querySelector({sel}); if (!el) return 'NOT_FOUND'; el.focus(); el.value = {escaped}; el.dispatchEvent(new Event('input', {{bubbles: true}})); return 'filled'; }})()"),
                super::super::engine::ACT_TIMEOUT,
            )?;
            Ok(expect(&r, "fill")?)
        }
        "press" => {
            let key = args
                .selector
                .as_deref()
                .ok_or_else(|| anyhow!("press requires selector=key name"))?;
            let key = js_string(key);
            let r = page.evaluate(
                &format!("(() => {{ const el = document.activeElement; if (!el) return 'NOT_FOUND'; el.dispatchEvent(new KeyboardEvent('keydown', {{key: {key}, bubbles: true}})); el.dispatchEvent(new KeyboardEvent('keyup', {{key: {key}, bubbles: true}})); return 'pressed'; }})()"),
                super::super::engine::ACT_TIMEOUT,
            )?;
            Ok(expect(&r, "press")?)
        }
        "select" => {
            let sel = selector_of("select")?;
            let text = args
                .value
                .as_ref()
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("select requires value (option value)"))?;
            let escaped = js_string(text);
            let r = page.evaluate(
                &format!("(() => {{ const el = document.querySelector({sel}); if (!el) return 'NOT_FOUND'; el.value = {escaped}; el.dispatchEvent(new Event('change', {{bubbles: true}})); return 'selected'; }})()"),
                super::super::engine::ACT_TIMEOUT,
            )?;
            Ok(expect(&r, "select")?)
        }
        "scroll" => {
            let dy = args.value.as_ref().and_then(Value::as_i64).unwrap_or(600);
            let r = page.evaluate(
                &format!("window.scrollBy(0, {dy}); 'scrolled'"),
                super::super::engine::ACT_TIMEOUT,
            )?;
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

#[cfg(test)]
mod tests {
    use super::js_string;

    #[test]
    fn js_string_escapes_what_a_literal_needs() {
        assert_eq!(js_string("plain"), "'plain'");
        assert_eq!(js_string("it's"), "'it\\'s'");
        assert_eq!(js_string("a\\b"), "'a\\\\b'");
        // The case that used to break a whole `act` call: a multi-line fill
        // value produced a literal newline inside the JS string → SyntaxError.
        assert_eq!(js_string("l1\nl2"), "'l1\\nl2'");
        assert_eq!(js_string("a\tb\r"), "'a\\tb\\r'");
        assert_eq!(js_string("\u{2028}"), "'\\u2028'");
        assert_eq!(js_string("\u{1}"), "'\\u0001'");
        // A selector containing a quote stays one literal.
        assert_eq!(js_string("[data-x='y']"), "'[data-x=\\'y\\']'");
        // CJK passes through untouched.
        assert_eq!(js_string("中文"), "'中文'");
    }
}
