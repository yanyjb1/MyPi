// A message body to HTML: one block's source is cut into the runs that paint on their own — prose, a fence, a table,
// a quote, a list — and a settled run never parses twice.

export function esc(text) {
  return String(text ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

const TEXT = "text";
const CODE = "code";
const TABLE = "table";
const QUOTE = "quote";
const LIST = "list";

// A run that a following run of the same kind simply continues, so the two are one run after all: a line half said
// has no run of its own, and the last settled run has to take it back.
const JOINT = new Set([TEXT, LIST, QUOTE]);

// Split one block's text into segments, in source order. `source` starts at character `base`, which every cut
// carries back so an append can resume at the tail it left open.
function splitFrom(source, base) {
  const rows = [];
  let at = 0;
  while (at < source.length) {
    const cut = source.indexOf("\n", at);
    if (cut < 0) {
      rows.push(source.slice(at));
      break;
    }
    rows.push(source.slice(at, cut + 1));
    at = cut + 1;
  }

  const out = [];
  let index = 0;
  let pos = base;
  const span = (from, to) => {
    let sum = 0;
    for (let i = from; i < to; i += 1) sum += rows[i].length;
    return sum;
  };

  while (index < rows.length) {
    const line = rows[index];
    if (!line.trim()) {
      pos += line.length;
      index += 1;
      continue;
    }
    const opening = fence(line);
    if (opening) {
      const [end, closed] = fenceEnd(rows, index + 1, opening.marker);
      const stop = closed ? end + 1 : rows.length;
      out.push({
        at: pos,
        seg: {
          kind: CODE,
          lang: opening.lang,
          source: rows.slice(index, stop).join(""),
        },
      });
      pos += span(index, stop);
      index = stop;
      continue;
    }
    if (tableHead(rows, index)) {
      let end = index + 2;
      while (end < rows.length && rows[end].includes("|")) end += 1;
      out.push({
        at: pos,
        seg: {
          kind: TABLE,
          lang: null,
          source: rows.slice(index, end).join(""),
        },
      });
      pos += span(index, end);
      index = end;
      continue;
    }
    const kind = quote(line) ? QUOTE : item(line) ? LIST : TEXT;
    let end = index + 1;
    while (end < rows.length && same(kind, rows, end)) end += 1;
    out.push({
      at: pos,
      seg: { kind, lang: null, source: rows.slice(index, end).join("") },
    });
    pos += span(index, end);
    index = end;
  }
  return out;
}

// Whether the line keeps the run of this kind going; a blank line never does.
function same(kind, rows, at) {
  const line = rows[at];
  if (!line.trim() || fence(line) || tableHead(rows, at)) return false;
  if (kind === QUOTE) return quote(line);
  // A list keeps its indented continuations, and an indented line never starts a paragraph of its own.
  if (kind === LIST) return item(line) || indent(line);
  return !quote(line) && !item(line) && !indent(line);
}

// A fence opener: the marker and its language.
function fence(line) {
  const trimmed = line.trimStart();
  for (const marker of ["```", "~~~"]) {
    if (trimmed.startsWith(marker)) {
      const lang = trimmed.slice(marker.length).split(/[ ,{]/)[0].trim();
      return { lang: lang || null, marker };
    }
  }
  return null;
}

// Where the fence closes: the row, and whether a closer was found at all.
function fenceEnd(rows, from, marker) {
  for (let i = from; i < rows.length; i += 1) {
    if (rows[i].trimStart().startsWith(marker)) return [i, true];
  }
  return [rows.length, false];
}

function quote(line) {
  return line.trimStart().startsWith(">");
}

// A list item, whatever its marker.
function item(line) {
  const trimmed = line.trimStart();
  const first = trimmed[0];
  if (first === "-" || first === "*" || first === "+") {
    return trimmed.slice(1).startsWith(" ");
  }
  if (first >= "0" && first <= "9") {
    const rest = trimmed.replace(/^[0-9]+/, "");
    return rest.startsWith(". ") || rest.startsWith(") ");
  }
  return false;
}

// A line that belongs to the line above rather than starting its own.
function indent(line) {
  return line.startsWith(" ") || line.startsWith("\t");
}

// A table header: this line has cells, and the next is a delimiter row.
function tableHead(rows, at) {
  const line = rows[at];
  if (line === undefined || !line.includes("|")) return false;
  const next = rows[at + 1];
  if (next === undefined) return false;
  return next.split("").every((c) => "|-: \t\n\r".includes(c)) &&
    next.includes("-");
}

// The lines of a run, newlines off.
const lines = (source) =>
  source.split("\n").filter((line, at, all) => at < all.length - 1 || line);

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

// One line of prose: the marks inside code spans stay literal, the rest become their tags.
function inline(source) {
  const spans = [];
  let out = esc(source).replace(/`([^`\n]+)`/g, (_, code) => {
    spans.push(code);
    return "\u0000" + (spans.length - 1) + "\u0000";
  });
  out = out
    .replace(/\*\*([^*\n]+)\*\*/g, "<strong>$1</strong>")
    .replace(/(^|[^*\w])\*([^*\n]+)\*/g, "$1<em>$2</em>")
    .replace(
      /\[([^\]\n]+)\]\((https?:[^)\s]+)\)/g,
      '<a href="$2" target="_blank" rel="noreferrer">$1</a>',
    );
  return out.replace(
    /\u0000(\d+)\u0000/g,
    (_, at) => "<code>" + spans[Number(at)] + "</code>",
  );
}

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
  // The open lists, outermost first. An item lands inside its parent's item, which is why a list opening needs
  // nothing closed first and an item at the same depth closes the one before it.
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

// A block's segments growing piece by piece: an append re-splits the open tail alone.
export class Text {
  constructor(source = "") {
    const cuts = splitFrom(source, 0);
    this.from = cuts.length ? cuts[cuts.length - 1].at : 0;
    // How much text has been read here. A piece that has said nothing new is not split again, which is what makes
    // a settled piece free: the running round re-reads only the piece it is still saying.
    this.len = source.length;
    // The segment, and the byte it starts at: a joint is only a joint where nothing came between.
    this.segs = cuts.map((cut) => cut.seg);
    this.ats = cuts.map((cut) => cut.at);
  }

  // One more piece landed: the tail is split again, its closed head joins the rest.
  grow(source) {
    if (source.length <= this.len) return false;
    this.len = source.length;
    // The open tail is opened again from where it starts; everything before it stays as it stands.
    const tail = this.tailIndex();
    this.segs.length = tail;
    this.ats.length = tail;
    let base = this.from;
    let cuts = splitFrom(source.slice(base), base);
    // A run that the next one continues is one run: a line half said has no run of its own, and the settled run
    // before it has to take the whole of it back.
    while (cuts.length) {
      const last = this.segs.length - 1;
      if (last < 0) break;
      const prev = this.segs[last];
      if (prev.kind !== cuts[0].seg.kind || !JOINT.has(prev.kind)) break;
      if (this.ats[last] + prev.source.length !== cuts[0].at) break;
      base = this.ats[last];
      this.segs.pop();
      this.ats.pop();
      cuts = splitFrom(source.slice(base), base);
    }
    if (!cuts.length) return false;
    this.from = cuts[cuts.length - 1].at;
    for (const cut of cuts) {
      this.segs.push(cut.seg);
      this.ats.push(cut.at);
    }
    return true;
  }

  // The open tail: the run a repaint starts at, everything before it settled.
  tailIndex() {
    return Math.max(0, this.segs.length - 1);
  }
}

export function body(source) {
  const text = new Text(String(source ?? "").replace(/\r\n?/g, "\n"));
  return text.segs.map(paint).join("");
}