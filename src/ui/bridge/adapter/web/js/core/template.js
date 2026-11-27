// The one way a node is made: a template the shell carries, cloned, and filled by the names it stamped on its own
// children. A panel brings its own templates and its own sheet; both are taken in the first time it is opened.

const docked = new Map();
const fetched = new Map();
const linked = new Set();

export const at = (id) => document.getElementById(id);

export function clear(node) {
  while (node.firstChild) node.removeChild(node.firstChild);
  return node;
}

// Every template a fragment carries joins the same shelf as the shell's own, so a panel's shapes are cloned the
// same way the page's shapes are.
function shelf(root) {
  for (const node of root.querySelectorAll("template[id]")) {
    docked.set(node.id, node.content);
    node.remove();
  }
}

function content(id) {
  if (!docked.has(id)) {
    const source = at(id);
    if (!source) throw new Error("no template named " + id);
    docked.set(id, source.content);
  }
  return docked.get(id);
}

// One clone of a template, as a fragment: a shape with more than one root is laid whole.
export function clone(id) {
  return content(id).cloneNode(true);
}

// One clone, as the element its template opens with.
export function build(id) {
  return clone(id).firstElementChild;
}

// One clone, as every element its template holds: a row of a grid, a pair of spans, a line and its parts.
export function parts(id) {
  return [...clone(id).children];
}

// A panel's own markup, by the address it lives at; a second ask for it is the first one's answer.
export function fetchTemplate(url) {
  if (!fetched.has(url)) {
    fetched.set(url, fetch(url).then((res) => {
      if (!res.ok) throw new Error("HTTP " + res.status + " on " + url);
      return res.text();
    }).then((html) => {
      const box = document.createElement("template");
      box.innerHTML = html.trim();
      shelf(box.content);
      return box.content;
    }));
  }
  return fetched.get(url).then((content) => content.cloneNode(true));
}

// The node a clone stamped with a name, wherever it stands inside it.
export function ref(node, name) {
  return node.querySelector('[data-ref="' + name + '"]');
}

// A named node says its text, and says nothing at all by standing out of the way when it has none to say.
export function text(node, name, said) {
  const target = ref(node, name);
  if (!target) return;
  const value = said == null ? "" : String(said);
  target.textContent = value;
  target.hidden = !value;
}

// Whether a named node stands at all, without touching what it holds.
export function show(node, name, on) {
  const target = ref(node, name);
  if (target) target.hidden = !on;
}

// A sheet a panel carries: the head holds one link per address, however many times a panel is opened.
export function stylesheet(url) {
  if (linked.has(url) || document.querySelector('link[href="' + url + '"]')) {
    linked.add(url);
    return;
  }
  linked.add(url);
  const link = document.createElement("link");
  link.rel = "stylesheet";
  link.href = url;
  document.head.append(link);
}