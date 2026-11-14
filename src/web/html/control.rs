//! The controls a Markdown document cannot express: what the page offers to click or to fill,
//! and the selector that reaches each one.

use anyhow::{Context as _, Result};
use serde_json::Value;

use crate::server::config::rules::Part;
use crate::web::artifact;

/// How many controls the answer lists inline. Past this the rest go to one file, so a reader
/// still has somewhere to look.
const LISTED: usize = 10;

/// The page-side half: one expression whose value is a JSON object. The placeholders are
/// filled by `expression`, so the script itself stays one readable literal.
const SCAN: &str = r#"(() => {
  const body = document.body;
  if (!body) return JSON.stringify({ text: 0, items: [] });
  const SCOPE = %SCOPE%;
  const SKIP = %SKIP%;
  const DIRECT = %DIRECT%;
  const NAMES = ['aria-label', 'title', 'alt', 'placeholder', 'value'];
  const clean = s => (s || '').replace(/\s+/g, ' ').trim();
  const clip = s => (s.length > 60 ? s.slice(0, 60) + '…' : s);

  const path = el => {
    const parts = [];
    let cur = el;
    for (let guard = 0; cur && cur.nodeType === 1 && guard < 40; guard++) {
      const tag = cur.tagName.toLowerCase();
      if (cur.id) { parts.unshift('#' + CSS.escape(cur.id)); break; }
      if (tag === 'body' || tag === 'html') { parts.unshift('body'); break; }
      const holder = cur.parentElement;
      if (!holder) { parts.unshift(tag); break; }
      const same = Array.from(holder.children).filter(c => c.tagName === cur.tagName);
      parts.unshift(same.length > 1 ? tag + ':nth-of-type(' + (same.indexOf(cur) + 1) + ')' : tag);
      cur = holder;
    }
    return parts.join(' > ');
  };

  const name = el => {
    for (const key of NAMES) {
      const value = clean(el.getAttribute(key));
      if (value) return clip(value);
    }
    return clip(clean(el.innerText || el.textContent));
  };

  // The nearest heading or label before the control, so a row of identical "Submit" buttons
  // still says which one is which.
  const context = el => {
    let cur = el;
    for (let hop = 0; cur && hop < 6; hop++) {
      let sib = cur.previousElementSibling;
      for (let step = 0; sib && step < 5; step++) {
        const found = sib.matches('h1,h2,h3,h4,h5,h6,label,legend,caption')
          ? sib
          : sib.querySelector('h1,h2,h3,h4,h5,h6,label,legend,caption');
        if (found) return clip(clean(found.innerText || found.textContent));
        sib = sib.previousElementSibling;
      }
      cur = cur.parentElement;
    }
    return '';
  };

  const kind = el => {
    const tag = el.tagName.toLowerCase();
    if (tag === 'input') {
      const type = (el.getAttribute('type') || 'text').toLowerCase();
      return ['submit', 'button', 'reset', 'checkbox', 'radio', 'image'].includes(type)
        ? 'button'
        : 'input:' + type;
    }
    return tag;
  };

  const roots = [];
  if (SCOPE.length === 0) {
    roots.push({ at: body, drop: [] });
  } else {
    for (const part of SCOPE) {
      const found = document.querySelector(part.selector);
      if (found) roots.push({ at: found, drop: part.drop });
    }
  }

  // A rule's drops belong to the region they were written for; a match above that region says
  // nothing about what is inside it.
  const skipped = (el, region, drops) => {
    if (el.closest(SKIP)) return true;
    return drops.some(selector => {
      const hit = el.closest(selector);
      return hit !== null && hit !== region && region.contains(hit);
    });
  };

  const seen = new Set();
  const items = [];
  const add = (el, region, drops) => {
    if (seen.has(el) || skipped(el, region, drops) || el.getClientRects().length === 0) return;
    seen.add(el);
    items.push({ select: path(el), kind: kind(el), name: name(el), context: context(el) });
  };

  for (const root of roots) {
    const region = root.at;
    const drops = root.drop;
    for (const el of region.querySelectorAll(DIRECT)) add(el, region, drops);
    for (const el of region.querySelectorAll('div, span, li, td')) {
      if (seen.has(el) || skipped(el, region, drops)) continue;
      if (getComputedStyle(el).cursor === 'pointer') add(el, region, drops);
    }
  }

  const text = roots.reduce((sum, root) => sum + clean(root.at.innerText).length, 0);
  return JSON.stringify({ text: text, items: items });
})()"#;

/// Elements the conversion drops, or that carry no prose a reader asked for, are not controls a
/// reader should be handed either: the list describes what survived.
const SKIP: &str = "footer, nav, aside, header, [aria-hidden=\"true\"]";

/// What a click may be meant for, by what the markup says. A handler attached from script
/// leaves no attribute behind, so the pointer cursor is scanned for as well: it is the one
/// trace it does leave.
const DIRECT: &str = "button, input, select, textarea, summary, [role=\"button\"], [onclick], \
                      a[href^=\"javascript:\"], a[href=\"#\"]";

/// One control, as the page answers for it.
struct Control {
    select: String,
    kind: String,
    name: String,
    context: String,
}

impl Control {
    /// One line, the same one inline and in the file the rest go to.
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

/// What the page offers, and whether it holds any text at all.
pub(crate) struct Operable {
    /// Trimmed length of the body's rendered text.
    pub(crate) text: usize,
    controls: Vec<Control>,
}

impl Operable {
    /// The section appended to the answer, or nothing when the page offers no control. The ones
    /// past the inline budget are written out, and the answer names where they went.
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

/// The scan expression. `regions` are the kept regions, each with the drops its rule wrote for it;
/// empty searches the whole body and drops nothing.
pub(crate) fn expression(regions: &[Part]) -> String {
    let scope: Vec<serde_json::Value> = regions
        .iter()
        .map(|part| serde_json::json!({"selector": part.selector(), "drop": part.drops()}))
        .collect();
    let scope = serde_json::to_string(&scope).expect("a rule's regions always serialize");
    let skip = serde_json::to_string(SKIP).expect("a selector list always serializes");
    let direct = serde_json::to_string(DIRECT).expect("a selector list always serializes");
    SCAN
        .replace("%SKIP%", &skip)
        .replace("%DIRECT%", &direct)
        .replace("%SCOPE%", &scope)
}

/// Read the answer: how much text the body held, and which controls it offered.
pub(crate) fn scan(answered: &str) -> anyhow::Result<Operable> {
    let value: Value = serde_json::from_str(answered).context("control scan")?;
    let number = |key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0) as usize;
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
    Ok(Operable {
        text: number("text"),
        controls,
    })
}
