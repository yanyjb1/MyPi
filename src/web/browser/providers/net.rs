//! Network capture — "find the API this page calls": ask the page for
//! its own performance-resource entries instead of tapping Network
//! events. Zero protocol surface, no event races.

use super::super::engine::BrowserArgs;
use crate::web::utils::session::Page;
use serde_json::Value;

pub(crate) fn capture_network(page: &Page, args: &BrowserArgs) -> anyhow::Result<String> {
    let filter = args.filter.clone().unwrap_or_default();
    let max = args.max.unwrap_or(20).clamp(1, 100);

    // Ask the page for its own performance entries: zero protocol surface,
    // no event races, and it covers every request the page actually made
    // (XHR/fetch/script/img) with initiatorType labels. The JS side returns a
    // generous window (`SCAN`) and the filter/take happen in Rust: slicing to
    // `max` *before* filtering under-reported matches whenever the newest
    // requests were not the matching ones.
    const SCAN: usize = 400;
    let expr = format!(
        "JSON.stringify((performance.getEntriesByType('resource')||[]).slice(-{SCAN}).map(e => ({{name: e.name, type: e.initiatorType, ms: Math.round(e.duration)}})))"
    );
    let raw = page.evaluate(&expr, super::super::engine::ACT_TIMEOUT)?;
    let entries: Vec<Value> = serde_json::from_str(&raw).unwrap_or_default();
    Ok(format_entries(&entries, &filter, max))
}

/// Render the captured requests: matching entries only, newest last, at most
/// `max` of them (the newest ones). Pure so the selection rule is testable
/// without a browser.
pub(crate) fn format_entries(entries: &[Value], filter: &str, max: usize) -> String {
    let matched: Vec<&Value> = entries
        .iter()
        .filter(|e| {
            filter.is_empty()
                || e.get("name")
                    .and_then(Value::as_str)
                    .is_some_and(|n| n.contains(filter))
        })
        .collect();
    let skip = matched.len().saturating_sub(max);
    let shown = matched.len() - skip;

    let mut out = String::from("network requests (newest last):\n");
    for e in matched.into_iter().skip(skip) {
        out.push_str(&format!(
            "  [{}] {} ({})\n",
            e.get("type").and_then(Value::as_str).unwrap_or("?"),
            e.get("name").and_then(Value::as_str).unwrap_or(""),
            e.get("ms").and_then(Value::as_i64).unwrap_or(0)
        ));
    }
    if shown == 0 {
        out.push_str(&format!("  (no requests matching {filter:?})\n"));
    }
    out
}

// --- read / screenshot -------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::format_entries;
    use serde_json::json;

    fn entry(name: &str) -> serde_json::Value {
        json!({ "name": name, "type": "xhr", "ms": 3 })
    }

    #[test]
    fn filtering_happens_before_the_limit() {
        // The old order took the newest `max` entries and *then* filtered, so a
        // matching request older than that window disappeared from the report.
        let entries: Vec<serde_json::Value> = (0..10)
            .map(|i| entry(&format!("https://api.test/keep/{i}")))
            .chain((0..50).map(|i| entry(&format!("https://cdn.test/img{i}"))))
            .collect();
        let out = format_entries(&entries, "/keep/", 3);
        let kept: Vec<&str> = out.lines().filter(|l| l.contains("api.test")).collect();
        assert_eq!(kept.len(), 3, "只留最新三条匹配项: {out}");
        assert!(out.contains("/keep/9"), "最新的匹配项必须在: {out}");
        assert!(!out.contains("cdn.test"), "不匹配的不该出现: {out}");
    }

    #[test]
    fn no_match_is_reported_explicitly() {
        let out = format_entries(&[entry("https://a/")], "zzz", 5);
        assert!(out.contains("no requests matching"), "{out}");
        // No filter: everything (up to max) is listed.
        let out = format_entries(&[entry("https://a/"), entry("https://b/")], "", 5);
        assert!(
            out.contains("https://a/") && out.contains("https://b/"),
            "{out}"
        );
    }
}
