// A message body to HTML, one segment at a time: a settled segment never parses twice.

import { paint } from "./paint.js";
import { LIST, QUOTE, splitFrom, TEXT } from "./segment.js";

// A run that a following run of the same kind simply continues, so the two are one run after all: a line
// half said has no run of its own, and the last settled run has to take it back.
const JOINT = new Set([TEXT, LIST, QUOTE]);

// A block's segments growing piece by piece: an append re-splits the open tail alone.
export class Text {
  constructor(source = "") {
    const cuts = splitFrom(source, 0);
    this.from = cuts.length ? cuts[cuts.length - 1].at : 0;
    // How much text has been read here. A piece that has said nothing new is not split again, which is what
    // makes a settled piece free: the running round re-reads only the piece it is still saying.
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
    // A run that the next one continues is one run: a line half said has no run of its own, and the settled
    // run before it has to take the whole of it back.
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
