//! Site rules: which part of a page is its prose, and which parts of that are not.
//!
//! The file lives beside `config.yaml` in the user's config directory. A machine that has none
//! is given the built-in set on first use; after that the file is the authority, and a site the
//! user deletes from it falls back to the generic conversion.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The file's name inside the config directory.
const FILE: &str = "web-rules.json";

/// One page pattern, and the parts of that page which are its prose.
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Rule {
    /// Matched against `host + path`; the first rule that matches is the one used. A regex.
    #[serde(rename = "match")]
    matcher: String,
    keep: Vec<Part>,
}

/// One kept region: a selector, and what to leave out of it.
#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Part {
    /// Searched from the page body; the first element it names is the region.
    selector: String,
    /// Removed from inside that region only.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    drop: Vec<String>,
}

/// The whole file: one array, ordered so a specific path may precede its site's catch-all.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(transparent)]
pub(crate) struct Rules(Vec<Rule>);

impl Rules {
    /// The file's own text, for the copy written on a machine that has none.
    fn text(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// The regions to keep for this page, in order, or nothing when no rule matches.
    pub(crate) fn regions(&self, host: &str, path: &str) -> &[Part] {
        let subject = format!("{host}{path}");
        self.0
            .iter()
            .find(|rule| matches(&rule.matcher, &subject))
            .map_or(&[], |rule| rule.keep.as_slice())
    }
}

impl Part {
    pub(crate) fn selector(&self) -> &str {
        &self.selector
    }

    pub(crate) fn drops(&self) -> &[String] {
        &self.drop
    }
}

/// A pattern that cannot be read matches nothing: a broken rule must not take the page with it.
fn matches(pattern: &str, subject: &str) -> bool {
    regex::Regex::new(pattern).is_ok_and(|re| re.is_match(subject))
}

/// Where the file lives: the config directory's root, beside `config.yaml`.
fn path() -> PathBuf {
    crate::server::config::paths::ConfigPaths::from_env()
        .global_root()
        .map_or_else(
            || crate::server::platform::env::config_dir().join(FILE),
            |root| root.join(FILE),
        )
}

/// The rules in force. A machine without the file is given the built-in set, once; after that
/// the file is what is read, whether or not it still matches what was written.
pub(crate) fn load() -> Rules {
    let file = path();
    if file.is_file() {
        return std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
    }
    let builtin = builtin();
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&file, builtin.text());
    builtin
}

/// What a new machine starts from.
fn builtin() -> Rules {
    serde_json::from_str(include_str!("rules.json")).unwrap_or_default()
}

/// The page-side half of a rule: keep these regions, drop these from inside each, and answer the
/// resulting HTML. A selector is looked for from the body, so a site that writes `<main>` says
/// `main`, and a site that does not names its own container.
pub(crate) fn expression(regions: &[Part]) -> String {
    let mut kept = String::new();
    for part in regions {
        if !kept.is_empty() {
            kept.push(',');
        }
        kept.push_str("{\"selector\":");
        kept.push_str(&literal(part.selector()));
        kept.push_str(",\"drop\":[");
        for (i, drop) in part.drops().iter().enumerate() {
            if i > 0 {
                kept.push(',');
            }
            kept.push_str(&literal(drop));
        }
        kept.push_str("]}");
    }
    format!(
        r#"(() => {{
  const parts = [{kept}];
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

/// One selector as a JavaScript string literal.
pub(crate) fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
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
