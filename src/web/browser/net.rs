use anyhow::Result;
use serde_json::Value;

use super::actions::ACTION_TIMEOUT;
use super::page::Page;
use super::session;
use crate::server::config::BrowserConfig;
use crate::web::WebArgs;

pub(crate) fn run(args: &WebArgs, cfg: &BrowserConfig, session: i64) -> Result<String> {
    let filter = args.filter.clone().unwrap_or_default();
    let max = args.max.unwrap_or(20).clamp(1, 100);
    session::with_work(session, cfg, args.reserve, |page: &Page| {
        capture_network(page, &filter, max)
    })
}

fn capture_network(page: &Page, filter: &str, max: usize) -> Result<String> {
    const SCAN: usize = 400;
    let expr = format!(
        "JSON.stringify((performance.getEntriesByType('resource')||[]).slice(-{SCAN}).map(e => ({{name: e.name, type: e.initiatorType, ms: Math.round(e.duration)}})))"
    );
    let raw = page.evaluate(&expr, ACTION_TIMEOUT)?;
    let entries: Vec<Value> = serde_json::from_str(&raw).unwrap_or_default();
    Ok(format_entries(&entries, filter, max))
}

fn format_entries(entries: &[Value], filter: &str, max: usize) -> String {
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
