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
  const MARKS = ['data-mypi-at', 'data-mypi-twin'];
  const IGNORE = 'data-mypi-ignore';
  const BREAKS = ['p', 'div', 'li', 'tr', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'section',
                  'table', 'dd', 'dt', 'td', 'th', 'blockquote', 'pre'];
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

  // A control is named by a mark the page never had, so its place in the copy is not what finds it.
  const marks = new Map();
  let painted = 0;
  const paint = el => {
    let mark = marks.get(el);
    if (mark === undefined) {
      mark = String(painted++);
      marks.set(el, mark);
      el.setAttribute(MARKS[0], mark);
    }
    return mark;
  };

  const twin_of = (copy, mark) => {
    for (const tag of MARKS) {
      const found = copy.querySelector('[' + tag + '="' + mark + '"]');
      if (found) return found;
    }
    return null;
  };

  // An element declared plaintified is read as its words: the subtree loses every element, block
  // boundaries stay as line breaks, and nothing inside it can be addressed again.
  const words = el => {
    const tokens = [];
    const apart = () => {
      const last = tokens[tokens.length - 1];
      if (last !== undefined && last !== null && last !== ' ') tokens.push(' ');
    };
    const walk = node => {
      for (const kid of Array.from(node.childNodes)) {
        if (kid.nodeType === 3) {
          const text = kid.nodeValue.replace(/\s+/g, ' ');
          if (text) tokens.push(text);
        } else if (kid.nodeType === 1) {
          const tag = kid.tagName.toLowerCase();
          if (tag === 'script' || tag === 'style' || tag === 'svg' || tag === 'mat-icon') continue;
          if (tag === 'br') { tokens.push(null); continue; }
          const block = BREAKS.includes(tag);
          if (block) tokens.push(null);
          else apart();
          walk(kid);
          if (block) tokens.push(null);
        }
      }
    };
    walk(el);
    const kept = [];
    for (const token of tokens) {
      if (token === null) {
        if (kept.length && kept[kept.length - 1] !== null) kept.push(null);
        continue;
      }
      if (token.trim() === '' && (kept.length === 0 || kept[kept.length - 1] === null)) continue;
      kept.push(token);
    }
    while (kept.length && kept[kept.length - 1] === null) kept.pop();
    return kept;
  };

  const tidy = s => s.replace(/\s+/g, ' ')
    .replace(/ ([,;.:!?%)\]}])/g, '$1')
    .replace(/([(\[{]) /g, '$1')
    .trim();

  const plain = el => {
    const lines = [[]];
    for (const token of words(el)) {
      if (token === null) {
        if (lines[lines.length - 1].length) lines.push([]);
      } else {
        lines[lines.length - 1].push(token);
      }
    }
    const out = document.createDocumentFragment();
    let wrote = false;
    for (const tokens of lines) {
      const text = tidy(tokens.join(''));
      if (text === '') continue;
      if (wrote) out.appendChild(document.createElement('br'));
      out.appendChild(document.createTextNode(text));
      wrote = true;
    }
    // A plaintified block ends the line it stood on, so the next block starts on its own.
    out.appendChild(document.createElement('br'));
    el.replaceWith(out);
  };

  const roots = [];
  if (SCOPE.length === 0) {
    roots.push({ at: body, drop: [], plain: [] });
  } else {
    for (const part of SCOPE) {
      const found = document.querySelector(part.selector);
      if (found) roots.push({ at: found, drop: part.drop, plain: part.plain });
    }
  }

  const html = [];
  const named = [];
  const pointed = [];
  let text = 0;
  for (const root of roots) {
    const region = root.at;
    for (const selector of root.drop) {
      for (const gone of region.querySelectorAll(selector)) gone.setAttribute(IGNORE, '1');
    }

    const seen = new Set();
    const wanted = [];
    const add = (el, direct) => {
      if (seen.has(el)) return;
      seen.add(el);
      paint(el);
      wanted.push({ el: el, direct: direct });
    };
    for (const el of region.querySelectorAll(DIRECT)) add(el, true);
    for (const el of region.querySelectorAll('div, span, li, td')) {
      if (getComputedStyle(el).cursor === 'pointer') add(el, false);
    }

    const copy = region.cloneNode(true);
    for (const gone of copy.querySelectorAll('[' + IGNORE + ']')) gone.remove();
    for (const selector of root.plain) {
      for (const hit of copy.querySelectorAll(selector)) plain(hit);
    }

    for (const one of wanted) {
      const twin = twin_of(copy, marks.get(one.el));
      if (!twin) continue;
      const item = { select: path(one.el), kind: kind(one.el), name: name(twin), context: context(twin) };
      if (one.direct) named.push(item);
      else pointed.push(item);
    }

    for (const el of copy.querySelectorAll('[' + MARKS[0] + ']')) el.removeAttribute(MARKS[0]);
    html.push(copy.outerHTML);
    text += clean(copy.innerText).length;
  }

  for (const el of marks.keys()) el.removeAttribute(MARKS[0]);
  for (const el of document.querySelectorAll('[' + IGNORE + ']')) el.removeAttribute(IGNORE);
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
        .map(|part| {
            serde_json::json!({
                "selector": part.selector(),
                "drop": part.drops(),
                "plain": part.plaintify(),
            })
        })
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
