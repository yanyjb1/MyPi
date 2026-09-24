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
    // (XHR/fetch/script/img) with initiatorType labels.
    let expr = format!(
        "JSON.stringify((performance.getEntriesByType('resource')||[]).slice(-{max}).map(e => ({{name: e.name, type: e.initiatorType, ms: Math.round(e.duration)}})))"
    );
    let raw = page.evaluate(&expr, super::super::engine::ACT_TIMEOUT)?;
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
