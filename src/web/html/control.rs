//! The controls a Markdown document cannot express: what the page offers to click or to fill,
//! and the selector that reaches each one.

use anyhow::Context as _;
use serde_json::Value;

/// How many controls the answer lists. Past this a page is mostly chrome, and the count of the
/// rest is the useful part.
const LISTED: usize = 40;

/// The page-side half: one expression whose value is a JSON object. The placeholders are
/// filled by `expression`, so the script itself stays one readable literal.
const SCAN: &str = r#"(() => {
  const body = document.body;
  if (!body) return JSON.stringify({ text: 0, total: 0, items: [] });
  const LIMIT = %LISTED%;
  const SCOPE = %SCOPE%;
  const SKIP = '%SKIP%';
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
    roots.push(body);
  } else {
    for (const selector of SCOPE) {
      const found = document.querySelector(selector);
      if (found) roots.push(found);
    }
  }

  const seen = new Set();
  const items = [];
  let total = 0;
  const add = el => {
    if (seen.has(el) || el.closest(SKIP) || el.getClientRects().length === 0) return;
    if (seen.size >= 400) return;
    seen.add(el);
    total++;
    if (items.length >= LIMIT) return;
    items.push({ select: path(el), kind: kind(el), name: name(el), context: context(el) });
  };

  for (const root of roots) {
    for (const el of root.querySelectorAll('%DIRECT%')) add(el);
    for (const el of root.querySelectorAll('div, span, li, td')) {
      if (seen.has(el) || el.closest(SKIP)) continue;
      if (getComputedStyle(el).cursor === 'pointer') add(el);
    }
  }

  const text = roots.reduce((sum, root) => sum + clean(root.innerText).length, 0);
  return JSON.stringify({ text: text, total: total, items: items });
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

/// What the page offers, and whether it holds any text at all.
pub(crate) struct Operable {
    /// Trimmed length of the body's rendered text.
    pub(crate) text: usize,
    controls: Vec<Control>,
    /// Controls the page offered in all, listed or not.
    total: usize,
}

impl Operable {
    /// The section appended to the answer, or nothing when the page offers no control.
    pub(crate) fn section(&self) -> String {
        if self.controls.is_empty() {
            return String::new();
        }
        let mut out = String::from("\n## operable\n\n");
        for control in &self.controls {
            out.push_str("- ");
            out.push_str(&control.kind);
            if !control.name.is_empty() {
                out.push_str(&format!(" {:?}", control.name));
            }
            if !control.context.is_empty() {
                out.push_str(&format!(" under {:?}", control.context));
            }
            out.push_str(&format!(" -> `{}`\n", control.select));
        }
        let rest = self.total.saturating_sub(self.controls.len());
        if rest > 0 {
            out.push_str(&format!("- {rest} more not listed\n"));
        }
        out
    }
}

/// The scan expression. `scope` names the regions to search, empty for the whole body.
pub(crate) fn expression(scope: &[String]) -> String {
    let mut list = String::new();
    for (i, selector) in scope.iter().enumerate() {
        if i > 0 {
            list.push(',');
        }
        list.push_str(&crate::web::rules::literal(selector));
    }
    SCAN.replace("%LISTED%", &LISTED.to_string())
        .replace("%SKIP%", SKIP)
        .replace("%DIRECT%", DIRECT)
        .replace("%SCOPE%", &format!("[{list}]"))
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
        total: number("total"),
        controls,
    })
}
