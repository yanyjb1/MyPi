// The one way a node is made: tag, a bag of attributes and handlers, and its children.

// A drawing node has to be born in the svg namespace, or the browser builds an element that paints nothing.
const SVG = new Set([
  "svg",
  "g",
  "path",
  "line",
  "circle",
  "rect",
  "ellipse",
  "polyline",
  "polygon",
  "text",
  "use",
]);

export function el(tag, props, ...kids) {
  const node = SVG.has(tag)
    ? document.createElementNS("http://www.w3.org/2000/svg", tag)
    : document.createElement(tag);
  for (const [key, value] of Object.entries(props || {})) {
    if (value == null || value === false) continue;
    if (key === "text") node.textContent = value;
    else if (key === "html") node.innerHTML = value;
    else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
    else node.setAttribute(key, value);
  }
  // Every depth, not one: a caller that builds children by mapping a list builds lists of lists, and a list
  // handed straight to `append` is not a node — it is the string "[object HTMLSpanElement]".
  for (const kid of kids.flat(Infinity)) {
    if (kid == null || kid === false) continue;
    node.append(
      kid instanceof Node ? kid : document.createTextNode(String(kid)),
    );
  }
  return node;
}

export const at = (id) => document.getElementById(id);

export function clear(node) {
  while (node.firstChild) node.removeChild(node.firstChild);
  return node;
}
