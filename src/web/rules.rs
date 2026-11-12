//! The page-side half of a site rule: keep these regions, drop these from inside each, and
//! answer the resulting HTML.

use crate::server::config::rules::Part;

/// The expression that collects the kept regions. A selector is looked for from the body, so a
/// site that writes `<main>` says `main`, and a site that does not names its own container.
pub(crate) fn expression(regions: &[Part]) -> String {
    let parts: Vec<serde_json::Value> = regions
        .iter()
        .map(|part| serde_json::json!({"selector": part.selector(), "drop": part.drops()}))
        .collect();
    let parts = serde_json::to_string(&parts).expect("a rule's parts always serialize");
    format!(
        r#"(() => {{
  const parts = {parts};
  const html = [];
  for (const part of parts) {{
    const found = document.querySelector(part.selector);
    if (!found) continue;
    const copy = found.cloneNode(true);
    for (const selector of part.drop) {{
      for (const gone of copy.querySelectorAll(selector)) gone.remove();
    }}
    html.push(copy.outerHTML);
  }}
  return JSON.stringify({{ html: html.join('\n') }});
}})()"#
    )
}

/// Read the page's answer: the kept regions as HTML, empty when none of them resolved.
pub(crate) fn kept_html(answered: &str) -> String {
    serde_json::from_str::<serde_json::Value>(answered)
        .ok()
        .and_then(|value| {
            value
                .get("html")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_default()
}
