// A message body's runs, each painted with no knowledge of its neighbours.

import { esc } from "../../base/note.js";
import { inline } from "./inline.js";
import { CODE, LIST, QUOTE, TABLE } from "./segment.js";

const HEADING = /^(#{1,3})\s+(.*)$/;
const BULLET = /^(\s*)[-*+]\s+(.*)$/;
const NUMBER = /^(\s*)\d+[.)]\s+(.*)$/;
const RULE = /^\s*([-*_])\1{2,}\s*$/;

function cells(line) {
  return line
    .trim()
    .replace(/^\|/, "")
    .replace(/\|$/, "")
    .split("|")
    .map((cell) => cell.trim());
}

// The lines of a run, newlines off.
const lines = (source) =>
  source.split("\n").filter((line, at, all) => at < all.length - 1 || line);

// Prose: headings, rules, and paragraphs, the plain lines of one gathered under one paragraph.
function prose(source) {
  const out = [];
  let para = [];
  const flushPara = () => {
    if (para.length) {
      out.push("<p>" + inline(para.join("\n")).replace(/\n/g, "<br>") + "</p>");
    }
    para = [];
  };
  for (const line of lines(source)) {
    if (!line.trim()) {
      flushPara();
      continue;
    }
    if (RULE.test(line)) {
      flushPara();
      out.push("<hr>");
      continue;
    }
    const heading = HEADING.exec(line);
    if (heading) {
      flushPara();
      const level = heading[1].length;
      out.push("<h" + level + ">" + inline(heading[2]) + "</h" + level + ">");
      continue;
    }
    para.push(line);
  }
  flushPara();
  return out.join("");
}

// A list: items nested by their own indent, their indented continuations kept on the item above.
function listing(source) {
  const nodes = [];
  for (const line of lines(source)) {
    if (!line.trim()) continue;
    const bullet = BULLET.exec(line);
    const number = NUMBER.exec(line);
    if (!bullet && !number) {
      if (nodes.length) nodes[nodes.length - 1].more.push(line.trim());
      continue;
    }
    nodes.push({
      depth: Math.min(Math.floor(((bullet || number)[1] || "").length / 2), 2),
      tag: bullet ? "ul" : "ol",
      said: (bullet || number)[2],
      more: [],
    });
  }

  const out = [];
  // The open lists, outermost first. An item lands inside its parent's item, which is why a list opening
  // needs nothing closed first and an item at the same depth closes the one before it.
  const open = [];
  const shut = (keep) => {
    while (open.length > keep) out.push("</li></" + open.pop() + ">");
  };
  for (const node of nodes) {
    const depth = Math.min(node.depth, open.length);
    shut(depth + 1);
    if (open.length === depth + 1) {
      // The list this item belongs to is already open: the item before it ends, unless the marker changed.
      if (open[depth] === node.tag) out.push("</li><li>");
      else {
        shut(depth);
        out.push("<" + node.tag + "><li>");
        open.push(node.tag);
      }
    } else {
      while (open.length < depth + 1) {
        out.push("<" + node.tag + "><li>");
        open.push(node.tag);
      }
    }
    out.push(inline(node.said));
    for (const more of node.more) out.push("<br>" + inline(more));
  }
  shut(0);
  return out.join("");
}

function quoted(source) {
  const said = lines(source).map((line) => line.replace(/^\s*>\s?/, ""));
  return "<blockquote>" + inline(said.join("\n")).replace(/\n/g, "<br>") +
    "</blockquote>";
}

function fenced(seg) {
  const rows = lines(seg.source);
  const said = rows.slice(
    1,
    rows.length > 1 && rows[rows.length - 1].trim()
      ? rows.length - 1
      : rows.length,
  );
  const lang = seg.lang ? ' data-lang="' + esc(seg.lang) + '"' : "";
  return "<pre" + lang + "><code>" + esc(said.join("\n")) + "</code></pre>";
}

function table(source) {
  const rows = lines(source).filter((line) => line.trim());
  const head = cells(rows[0]);
  const body = rows.slice(2).map(cells);
  return "<table><thead><tr>" +
    head.map((cell) => "<th>" + inline(cell) + "</th>").join("") +
    "</tr></thead><tbody>" +
    body.map((row) =>
      "<tr>" + row.map((cell) => "<td>" + inline(cell) + "</td>").join("") +
      "</tr>"
    ).join("") +
    "</tbody></table>";
}

// One segment's HTML: the run painted on its own, with no knowledge of its neighbours.
export function paint(seg) {
  switch (seg.kind) {
    case CODE:
      return fenced(seg);
    case TABLE:
      return table(seg.source);
    case QUOTE:
      return quoted(seg.source);
    case LIST:
      return listing(seg.source);
    default:
      return prose(seg.source);
  }
}
