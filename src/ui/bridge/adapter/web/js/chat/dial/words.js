// A round's words: one line per notch, laid on that notch's own line and running inward from its tip.

import { el } from "../../base/dom.js";
import { call } from "../../base/wire.js";
import { state } from "../../base/state.js";
import { jump } from "../history/place.js";
import { loaded, said } from "../history/rounds.js";

// The ring in the svg's own units: the centre every line is hung from, and the room a line has.
const CY = 100;
const LENGTH = 58;
const FONT = 10.5;
const TAIL = "…";

// Widths are measured in em, never in pixels and never assumed: an em is the same in the drawing's units
// whatever size the ring is on screen, and a canvas measure knows that a wide glyph and a latin one differ.
const canvas = document.createElement("canvas");
const ctx = canvas.getContext("2d");

function prepare() {
  if (!ctx) return;
  const family = getComputedStyle(document.body).fontFamily || "sans-serif";
  ctx.font = "100px " + family;
}

// The width of a line in em.
function width(text) {
  if (!ctx) return text.length;
  return ctx.measureText(text).width / 100;
}

// The room a line has, in em.
const ROOM = LENGTH / FONT;

// The opening that fits the room: the rest is said with an ellipsis, and the whole message waits in the title.
// A width only ever grows with the text, so the cut is found by halving, never by measuring a glyph at a time.
function fitted(text) {
  const whole = String(text || "").replace(/\s+/g, " ").trim();
  if (!whole || width(whole) <= ROOM) return whole;
  const tail = width(TAIL);
  let low = 0;
  let high = whole.length;
  while (low < high) {
    const mid = Math.ceil((low + high) / 2);
    if (width(whole.slice(0, mid)) + tail > ROOM) high = mid - 1;
    else low = mid;
  }
  // A cut inside a surrogate pair is not a character: the pair goes whole or not at all.
  let cut = whole.slice(0, low);
  if (/[\uD800-\uDBFF]$/.test(cut)) cut = cut.slice(0, -1);
  return (cut || "") + TAIL;
}

// Each round's first words, kept here because the transcript's own page drops what it is not showing. Only the
// rounds the ring has actually shown live here, and only a few of them: a session of thirty thousand rounds
// must never be carried about in the browser.
let spoken = new Map();
const KEEP = 512;
// The session the kept words belong to: another one starts the map again.
let owner = null;
// Rounds already asked for, so a frame that lands mid-ask is not asked for twice.
const owed = new Set();

function remember(turn, text) {
  spoken.set(turn, text);
  if (spoken.size > KEEP) {
    const first = spoken.keys().next().value;
    spoken.delete(first);
  }
}

// One line: its anchor is the point where its own tick stops, and the turn is taken about that same point, so
// the words continue the tick's line exactly. Every line is pinned to one length, which makes all five tips end
// at the same radius whichever words they carry.
// Horizontal, parallel text layout with stepwise rightward indentation.
function line(round, angle, offset, text, isSelected) {
  const k = offset || 0;
  // Strictly horizontal: vertical alignment around CY = 100
  const y = CY + k * 20;
  // Stepwise rightward indent: center (k=0) is farthest left (x=16); edges indent rightward
  const indent = Math.abs(k) * 16;
  const x = 16 + indent;
  const displayText = text ||
    (round.compact ? "compact" : "turn " + round.turn);

  const node = el("text", {
    class: "word",
    x,
    y: y + 3,
    "text-anchor": "start",
    "font-size": FONT,
    "data-turn": String(round.turn),
    "data-on": isSelected ? "1" : "0",
    text: fitted(displayText),
  });
  node.title = displayText;
  node.addEventListener("click", (event) => {
    event.stopPropagation();
    jump(round.turn);
  });
  return node;
}

// The user's own words among the blocks one answer brought back, whichever interval of the round holds them.
function spokenOf(blocks, ranges) {
  const first = blocks.find((block) => {
    if (block.kind !== "user") return false;
    return (ranges || []).some(([lo, hi]) => lo <= block.id && block.id <= hi);
  });
  if (!first) return "";
  return String(first.prefix || "") + String(first.content || first.text || "");
}

// Batch prefetch missing round user texts with margin, avoiding individual requests or fetching all 8M rounds.
async function speakBatch(held, missingRounds, again) {
  if (!missingRounds.length) return;
  const targetRanges = [];
  const turnRounds = new Map();

  for (const r of missingRounds) {
    turnRounds.set(r.turn, r);
    for (const range of r.ranges || []) {
      targetRanges.push(range);
    }
  }

  if (!targetRanges.length) return;

  try {
    const answer = await call("blocks", {
      session: held.id,
      ranges: targetRanges,
    });
    const blocks = answer.blocks || [];
    for (const block of blocks) {
      if (held.blocks) held.blocks.set(block.id, block);
    }
    for (const [turn, r] of turnRounds.entries()) {
      const text = spokenOf(blocks, r.ranges) || ("turn " + turn);
      remember(turn, text);
    }
  } catch {}

  for (const r of missingRounds) owed.delete(held.id + ":" + r.turn);
  if (!state.session || state.session.id !== held.id) return;
  again();
}

const DIAL_RUNWAY = 64;
let inFlight = false;

// Every line of the window on its own tick's line, all of one length. Checks 64-round runway in scroll direction.
export function label(box, held, items, again, dir = 0, here = -1) {
  prepare();
  if (owner !== held.id) {
    owner = held.id;
    spoken = new Map();
  }
  const list = (held && held.structure) || [];
  const atIdx = here >= 0
    ? here
    : (items[0] ? list.indexOf(items[0].round) : 0);

  // The runway runs one way: the way the reader is turning the ring.
  const from = dir <= 0 ? Math.max(0, atIdx - DIAL_RUNWAY) : atIdx;
  const to = dir >= 0 ? Math.min(list.length - 1, atIdx + DIAL_RUNWAY) : atIdx;
  const missing = [];

  for (let i = from; i <= to; i += 1) {
    const round = list[i];
    if (!round || spoken.has(round.turn)) continue;
    if (loaded(held, round)) spoken.set(round.turn, said(held, round));
    else if (!owed.has(held.id + ":" + round.turn)) {
      owed.add(held.id + ":" + round.turn);
      missing.push(round);
    }
  }

  // The five beside the middle are read before the runway, so a notch that is on screen never waits for one.
  for (const { round } of items) {
    if (!spoken.has(round.turn) && loaded(held, round)) {
      spoken.set(round.turn, said(held, round));
    }
  }

  if (missing.length && !inFlight) {
    inFlight = true;
    speakBatch(held, missing, () => {
      inFlight = false;
      again();
    });
  }

  for (const { round, angle, offset, isSelected } of items) {
    box.append(line(round, angle, offset, spoken.get(round.turn), isSelected));
  }
}
