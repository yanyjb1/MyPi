// The page-side half of a read: apply a site rule once, answer the kept HTML and the controls in it.

use anyhow::{Context as _, Result};
use serde_json::Value;

use crate::server::config::rules::Part;
use crate::web::artifact;

// How many controls the answer lists inline; the rest go to one file a reader can still open.
const LISTED: usize = 10;

// One expression whose value is a JSON object; `expression` fills the placeholders.
const SCAN: &str = r#"(() => {
  const body = document.body;
  if (!body) return JSON.stringify({ text: 0, html: '', items: [] });
  const SCOPE = %SCOPE%;
  const DIRECT = %DIRECT%;
  const NAMES = ['aria-label', 'title', 'alt', 'placeholder', 'value'];
  const clean = s => (s || '').replace(/\s+/g, ' ').trim();
  const clip = s => (s.length > 60 ? s.slice(0, 60) + '…' : s);

  const path = el => {
    const parts = [];
    let cur = el;
    for (let guard = 0; cur && cur.nodeType === 1 && guard < 40; guard++) {
      const tag = cur.tagName.toLowerCase();
      if (cur.id) {
        parts.unshift('#' + CSS.escape(cur.id));
        if (document.querySelectorAll('#' + CSS.escape(cur.id)).length === 1) break;
      }
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

  // The nearest heading or label before the control, so a row of identical buttons still says which is which.
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

  const html = [];
  const named = [];
  const pointed = [];
  let text = 0;
  for (const root of roots) {
    const region = root.at;
    const copy = region.cloneNode(true);
    for (const selector of root.drop) {
      for (const gone of copy.querySelectorAll(selector)) gone.remove();
    }
    html.push(copy.outerHTML);
    text += clean(copy.innerText).length;

    const seen = new Set();
    const add = el => {
      if (seen.has(el)) return;
      seen.add(el);
      const select = path(el);
      const twin = copy.querySelector(select);
      if (!twin) return;
      const item = { select: select, kind: kind(el), name: name(twin), context: context(twin) };
      if (el.matches(DIRECT)) named.push(item);
      else pointed.push(item);
    };
    for (const el of region.querySelectorAll(DIRECT)) add(el);
    for (const el of region.querySelectorAll('div, span, li, td')) {
      if (getComputedStyle(el).cursor === 'pointer') add(el);
    }
  }
  return JSON.stringify({ text: text, html: html.join('\n'), items: named.concat(pointed) });
})()"#;

// What a click may be meant for. A handler from script leaves no attribute, so the pointer cursor counts too.
const DIRECT: &str = "button, input, select, textarea, summary, [role=\"button\"], [onclick], \
                      a[href^=\"javascript:\"], a[href=\"#\"]";

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
        .map(|part| serde_json::json!({"selector": part.selector(), "drop": part.drops()}))
        .collect();
    let scope = serde_json::to_string(&scope).expect("a rule's regions always serialize");
    let direct = serde_json::to_string(DIRECT).expect("a selector list always serializes");
    SCAN.replace("%DIRECT%", &direct).replace("%SCOPE%", &scope)
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
