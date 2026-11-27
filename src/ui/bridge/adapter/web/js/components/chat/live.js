// The running round's own pieces: each node built once, and only the piece that grew laid again. Every piece comes
// from a template the shell carries; a stream frame only ever re-reads what it already has.

import { build, text } from "../../core/template.js";
import { state } from "../../core/store.js";
import { Text, numberPills, paint } from "../../render/markdown.js";
import { card } from "../../render/tools.js";
import { noteLine } from "./note.js";

// A piece's node, built once and kept.
function viewOf(item) {
  switch (item.type) {
    case "user": {
      const node = build("tpl-you");
      text(node, "said", item.text);
      return { node };
    }
    case "text":
      return {
        node: build("tpl-prose"),
        parts: new Text(""),
        chunks: [],
        cursor: false,
      };
    case "thinking": {
      const node = build("tpl-think");
      return {
        node,
        parts: new Text(""),
        chunks: [],
        host: node.querySelector('[data-ref="body"]'),
      };
    }
    case "break":
      return { node: build("tpl-break") };
    case "wake":
      return { node: noteLine("notice", "wake", item.text) };
    case "notice":
      return { node: noteLine(item.level, "notice", item.text) };
    default:
      return { node: null, rev: null };
  }
}

// Paint a growing text into its host: the settled runs stay, the open tail alone is replaced. The host is the
// element the paragraph margins apply to, so a run laid into a box would lose them: it is laid here instead.
function paintInto(host, parts, chunks) {
  const tail = parts.tailIndex();
  for (let i = chunks.length - 1; i >= tail; i -= 1) {
    for (const node of chunks[i]) node.remove();
    chunks.pop();
  }
  for (let i = chunks.length; i < parts.segs.length; i += 1) {
    const probe = document.createElement("div");
    probe.innerHTML = paint(parts.segs[i]);
    const made = [...probe.childNodes];
    for (const node of made) host.append(node);
    chunks.push(made);
  }
}

// One piece brought up to date; `isTail` is the run the cursor sits on.
function sync(item, isTail) {
  const view = item.view || (item.view = viewOf(item));
  if (item.type === "text" || item.type === "thinking") {
    if (view.parts.grow(item.text || "")) {
      paintInto(view.host || view.node, view.parts, view.chunks);
    }
    if (item.type === "text" && isTail !== view.cursor) {
      view.cursor = isTail;
      view.node.classList.toggle("cursor", isTail);
    }
    // A thought stands open while the round runs and folds away once it lands, as it always has.
    if (item.type === "thinking") view.node.open = state.streaming;
  } else if (item.type === "tool") {
    if (!view.node || view.rev !== item.rev) {
      const made = card(item.name, item.args, item.intent, item.status, item.result, item.took, item.note);
      if (view.node) view.node.replaceWith(made);
      view.node = made;
      view.rev = item.rev;
    }
  }
  return view.node;
}

// The live pieces: the round being said now, before it lands as blocks.
export function liveTurn(held) {
  const turn = held.liveNode || (held.liveNode = build("tpl-live"));
  const items = held.live;
  // Only the last text run carries the cursor: the round is still being said, not each paragraph.
  let tail = -1;
  for (let i = 0; i < items.length; i += 1) {
    if (items[i].type === "text") tail = i;
  }
  const kept = new Set();
  for (let i = 0; i < items.length; i += 1) {
    const node = sync(items[i], i === tail);
    if (!node) continue;
    kept.add(node);
    if (turn.children[i] !== node) {
      turn.insertBefore(node, turn.children[i] || null);
    }
  }
  // A piece the round took back — a message that never left — leaves its node behind by identity, not by position:
  // the pieces after it have moved down a place.
  for (const node of [...turn.children]) {
    if (!kept.has(node)) node.remove();
  }
  // The running round settles too, so a page it cites twice wears the number the first of them was given.
  numberPills(turn);
  return turn;
}
