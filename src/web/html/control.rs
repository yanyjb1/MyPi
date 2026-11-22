// The page-side half of a read: apply a site rule once, answer the kept HTML and the controls in it.

use anyhow::{Context as _, Result};
use serde_json::Value;

use crate::server::config::rules::Part;
use crate::web::artifact;

// How many controls the answer lists inline; the rest go to one file a reader can still open.
const LISTED: usize = 10;

// The page-side scan; `expression` fills its one placeholder.
const SCAN: &str = include_str!("control.js");

// One control, as the page answers for it.
struct Control {
    select: String,
    kind: String,
    name: String,
    context: String,
}

impl Control {
    // One line, the same one inline and in the file the rest go to.
    fn line(&self) -> String {
        let mut out = String::from("- ");
        out.push_str(&self.kind);
        if !self.name.is_empty() {
            out.push_str(&format!(" {:?}", self.name));
        }
        if !self.context.is_empty() {
            out.push_str(&format!(" under {:?}", self.context));
        }
        out.push_str(&format!(" -> `{}`\n", self.select));
        out
    }
}

// What the page holds once the rule has been applied, and what it offers.
pub(crate) struct Scan {
    // Trimmed length of the kept regions' rendered text.
    pub(crate) text: usize,
    // The kept regions as HTML, every drop removed.
    pub(crate) html: String,
    controls: Vec<Control>,
}

impl Scan {
    // The section appended to the answer, nothing when no control survives; the rest go to a file.
    pub(crate) fn section(&self, site: &str) -> Result<String> {
        if self.controls.is_empty() {
            return Ok(String::new());
        }
        let mut out = String::from("\n## operable\n\n");
        let (listed, rest) = self.controls.split_at(self.controls.len().min(LISTED));
        for control in listed {
            out.push_str(&control.line());
        }
        if !rest.is_empty() {
            let body: String = rest.iter().map(Control::line).collect();
            let path = artifact::save(site, "md", body.as_bytes())?;
            out.push_str(&format!(
                "\n{} more controls: {}\n",
                rest.len(),
                path.display()
            ));
        }
        Ok(out)
    }
}

// The scan expression. `regions` are the kept regions with their drops; empty reads the whole body.
pub(crate) fn expression(regions: &[Part]) -> String {
    let scope: Vec<serde_json::Value> = regions
        .iter()
        .map(|part| {
            serde_json::json!({
                "selector": part.selector(),
                "drop": part.drops(),
                "plain": part.plaintify(),
            })
        })
        .collect();
    let scope = serde_json::to_string(&scope).expect("a rule's regions always serialize");
    SCAN.replace("%SCOPE%", &scope)
}

// Read the answer: the kept HTML, how much text it holds, and which controls it offers.
pub(crate) fn scan(answered: &str) -> anyhow::Result<Scan> {
    let value: Value = serde_json::from_str(answered).context("page scan")?;
    let number = |key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0) as usize;
    let text = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let field = |item: &Value, key: &str| {
        item.get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let controls: Vec<Control> = value
        .get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| Control {
                    select: field(item, "select"),
                    kind: field(item, "kind"),
                    name: field(item, "name"),
                    context: field(item, "context"),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Scan {
        text: number("text"),
        html: text("html"),
        controls,
    })
}
