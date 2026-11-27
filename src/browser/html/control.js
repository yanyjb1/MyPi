(() => {
  const body = document.body;
  if (!body) return JSON.stringify({ text: 0, html: '', items: [] });
  const SCOPE = %SCOPE%;
  const DIRECT = 'button, input, select, textarea, summary, [role="button"], [onclick], ' +
                 'a[href^="javascript:"], a[href="#"]';
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

    // One chain answers once: a real control beats the shells around it, and bare shells answer the innermost.
    const candidate = new Map(wanted.map(one => [one.el, one]));
    const beaten = new Set();
    for (const one of wanted) {
      for (let cur = one.el.parentElement, hop = 0; cur && hop < 40; cur = cur.parentElement, hop++) {
        const outer = candidate.get(cur);
        if (!outer) continue;
        const outer_loses = one.direct === outer.direct || !outer.direct;
        beaten.add(outer_loses ? outer.el : one.el);
        break;
      }
    }

    // A bare shell that holds a widget answers by that widget's control, so only the control is listed.
    const holds_control = el =>
      Array.from(el.querySelectorAll(DIRECT)).some(hit => !hit.closest('[' + IGNORE + ']'));
    const husk = one => {
      if (one.direct) return false;
      for (let cur = one.el.parentElement, hop = 0; cur && cur !== region && hop < 40; cur = cur.parentElement, hop++) {
        if (holds_control(cur)) return true;
      }
      return false;
    };

    const copy = region.cloneNode(true);
    for (const gone of copy.querySelectorAll('[' + IGNORE + ']')) gone.remove();
    for (const selector of root.plain) {
      for (const hit of copy.querySelectorAll(selector)) plain(hit);
    }

    for (const one of wanted) {
      if (beaten.has(one.el) || husk(one)) continue;
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
})()