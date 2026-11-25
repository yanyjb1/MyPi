// One block's source cut into the runs that paint on their own: prose, a fence, a table, a quote, a list.

export const TEXT = "text";
export const CODE = "code";
export const TABLE = "table";
export const QUOTE = "quote";
export const LIST = "list";

// Split one block's text into segments, in source order. `source` starts at character `base`, which every cut
// carries back so an append can resume at the tail it left open.
export function splitFrom(source, base) {
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
