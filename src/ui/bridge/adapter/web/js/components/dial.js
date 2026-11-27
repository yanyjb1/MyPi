// The dial: a half ring of the session's rounds at the right edge of the transcript, folded until a pointer wakes it.

import { at, clear } from "../core/template.js";
import { words } from "../core/api.js";
import { state } from "../core/store.js";
import { jump } from "./chat/scroll.js";
import { said } from "./chat/sheet.js";
import { loaded } from "./chat/window.js";

// The arc a notch sits on, in the svg's own units: centre (99,100), radius 96, bulging left. Bearings run negative
// along the sweep, so the middle of the ring is -180: the round in view sits straight out from the hub.
const CX = 99;
const CY = 100;
const R = 96;
const MID = -180;
const STEP = 30;
// A half ring holds seven bearings; the topmost and the bottommost are left empty, because a line there would have
// no room to say anything. Five it is, and the round in view keeps the middle one whatever its place in the list.
const SPAN = 2;
// How long a pointer rests on the ring before it is awake, how far a wheel must travel for one notch, and how far a
// finger must. A trackpad says dozens of small deltas where a mouse says one large one, so both are added up rather
// than taken one event at a time.
const WAIT = 180;
const SPIN = 24;
// How long the wheel must be still before what it has travelled so far is forgotten.
const REST = 220;
// How far a finger moves before it is dragging the ring rather than tapping the notch or the word it landed on.
const SLIP = 8;
// How far a finger travels for one notch: a drag is a coarser thing than a wheel, so it asks for more of it.
const PULL = 28;

let active = false;
let waking = 0;
let cursorIndex = -1;
let dir = 0;
// The travel added up since the last notch, the pause that ends one turn of the wheel, and the finger turning the ring.
let spun = 0;
let idle = 0;
let grab = null;
let dragged = false;

const rounds = () => (state.session && state.session.structure) || [];

function point(angle, radius) {
  const rad = (angle * Math.PI) / 180;
  return [CX + radius * Math.cos(rad), CY + radius * Math.sin(rad)];
}

// The position of the round in view or the standalone cursor while wandering the dial.
function bearing() {
  const list = rounds();
  if (cursorIndex >= 0 && cursorIndex < list.length) return cursorIndex;
  const seen = state.session && state.session.anchor;
  const found = list.findIndex((round) => round.turn === seen);
  return found < 0 ? Math.max(0, list.length - 1) : found;
}

// One notch, drawn from the track inwards: a tick that is also the way to its round.
function notch(round, angle, span) {
  const [x1, y1] = point(angle, R + 4);
  const [x2, y2] = point(angle, R + 4 - span);
  const tick = document.createElementNS("http://www.w3.org/2000/svg", "line");
  tick.setAttribute("class", "tick");
  tick.setAttribute("x1", x1);
  tick.setAttribute("y1", y1);
  tick.setAttribute("x2", x2);
  tick.setAttribute("y2", y2);
  tick.dataset.turn = String(round.turn);
  tick.setAttribute("vector-effect", "non-scaling-stroke");
  tick.addEventListener("click", (event) => {
    event.stopPropagation();
    fold();
    jump(round.turn);
  });
  return tick;
}

// The ring in the svg's own units: the centre every line is hung from, and the room a line has.
const LENGTH = 58;
const FONT = 10.5;
const TAIL = "…";

// Widths are measured in em, never in pixels and never assumed: an em is the same in the drawing's units whatever
// size the ring is on screen, and a canvas measure knows that a wide glyph and a latin one differ.
const canvas = document.createElement("canvas");
const ctx = canvas.getContext("2d");

function prepare() {
  if (!ctx) return;
  const family = getComputedStyle(document.body).fontFamily || "sans-serif";
  ctx.font = "100px " + family;
}

// The width of a line in em, and the room a line has.
function width(text) {
  if (!ctx) return text.length;
  return ctx.measureText(text).width / 100;
}

const ROOM = LENGTH / FONT;

// The opening that fits the room: the rest is said with an ellipsis, and the whole message waits in the title. A
// width only ever grows with the text, so the cut is found by halving, never by measuring a glyph at a time.
function fitted(text) {
  const whole = String(text || "")
    .replace(/\s+/g, " ")
    .trim();
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
// rounds the ring has actually shown live here, and only a few of them: a session of thirty thousand rounds must
// never be carried about in the browser.
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

// One line, hung on its own notch's line: every line is pinned to one length, so all five tips end at the same
// radius whichever words they carry.
function line(round, offset, text, isSelected) {
  const k = offset || 0;
  // Strictly horizontal: vertical alignment around CY = 100.
  const y = CY + k * 20;
  // Stepwise rightward indent: the middle is farthest left, the edges indent rightward.
  const indent = Math.abs(k) * 16;
  const x = 16 + indent;
  const display = text || (round.compact ? "compact" : "turn " + round.turn);

  const node = document.createElementNS("http://www.w3.org/2000/svg", "text");
  node.setAttribute("class", "word");
  node.setAttribute("x", x);
  node.setAttribute("y", y + 3);
  node.setAttribute("text-anchor", "start");
  node.setAttribute("font-size", FONT);
  node.dataset.turn = String(round.turn);
  node.dataset.on = isSelected ? "1" : "0";
  node.textContent = fitted(display);
  node.title = display;
  node.addEventListener("click", (event) => {
    event.stopPropagation();
    fold();
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

// A runway of rounds is read in one question, never one question a round.
async function speakBatch(held, missing, again) {
  if (!missing.length) return;
  const targetRanges = [];
  const turnRounds = new Map();
  for (const r of missing) {
    turnRounds.set(r.turn, r);
    for (const range of r.ranges || []) targetRanges.push(range);
  }
  if (!targetRanges.length) return;
  try {
    const answer = await words({ session: held.id, ranges: targetRanges });
    const blocks = answer.blocks || [];
    for (const block of blocks) {
      if (held.blocks) held.blocks.set(block.id, block);
    }
    for (const [turn, r] of turnRounds.entries()) {
      remember(turn, spokenOf(blocks, r.ranges) || "turn " + turn);
    }
  } catch {}

  for (const r of missing) owed.delete(held.id + ":" + r.turn);
  if (!state.session || state.session.id !== held.id) return;
  again();
}

const RUNWAY = 64;
let inFlight = false;

// Every line of the window on its own notch's line, all of one length. The runway runs the way the reader turns.
export function label(box, held, items, again, direction = 0, here = -1) {
  prepare();
  if (owner !== held.id) {
    owner = held.id;
    spoken = new Map();
  }
  const list = (held && held.structure) || [];
  const atIdx = here >= 0 ? here : items[0] ? list.indexOf(items[0].round) : 0;
  const from = direction <= 0 ? Math.max(0, atIdx - RUNWAY) : atIdx;
  const to = direction >= 0 ? Math.min(list.length - 1, atIdx + RUNWAY) : atIdx;
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

  for (const { round, offset, isSelected } of items) {
    box.append(line(round, offset, spoken.get(round.turn), isSelected));
  }
}

function paint() {
  const box = clear(at("dial-ticks"));
  clear(at("dial-words"));
  const wrap = at("dial-wrap");
  wrap.dataset.on = active ? "1" : "0";
  wrap.dataset.state = active ? "active" : "inactive";
  const held = state.session;
  const list = rounds();
  if (!list.length) return;
  const here = bearing();
  const items = [];
  for (let k = -SPAN; k <= SPAN; k += 1) {
    const round = list[here + k];
    if (!round) continue;
    const angle = MID - k * STEP;
    box.append(notch(round, angle, active ? 16 : 10));
    items.push({ round, offset: k, isSelected: k === 0 });
  }
  // The selected notch stays lit.
  for (const tick of box.children) {
    tick.dataset.on = Number(tick.dataset.turn) === list[here].turn ? "1" : "0";
  }
  if (active && held) label(at("dial-words"), held, items, paint, dir, here);
}

function wake() {
  if (active) return;
  active = true;
  cursorIndex = -1;
  dir = 0;
  paint();
}

function fold() {
  if (!active) return;
  active = false;
  cursorIndex = -1;
  dir = 0;
  // A ring shut mid-drag leaves no hand holding it: the pointer that would let go is not coming back.
  grab = null;
  spun = 0;
  clearTimeout(idle);
  paint();
}

// The wheel wanders within the dial's cursor only: the transcript is not scrolled and no body is fetched.
function wheel(event) {
  if (!active) return;
  event.preventDefault();
  event.stopPropagation();
  let travel = event.deltaY;
  if (!travel) return;
  // A line and a page are not pixels: each is turned into the distance it is worth before it is counted.
  if (event.deltaMode === 1) travel *= 16;
  else if (event.deltaMode === 2) travel *= 100;
  step(travel);
}

// Travel of either kind, in pixels: one way adds up, the other starts the count again, and a notch is taken per enough
// of it.
function step(travel, need = SPIN) {
  const list = rounds();
  if (list.length < 2 || !travel) return;
  const way = travel > 0 ? 1 : -1;
  if (way !== dir) {
    dir = way;
    spun = 0;
  }
  spun += Math.abs(travel);
  if (spun < need) return;
  // One notch per call, and the count starts again: a wheel that says a hundred pixels at a time is one notch, not
  // four, and a trackpad's many small ones add up until they are worth one.
  spun = 0;
  clearTimeout(idle);
  idle = setTimeout(() => {
    spun = 0;
    dir = 0;
  }, REST);
  const here = bearing();
  const there = Math.min(Math.max(here + dir, 0), list.length - 1);
  if (there !== here) {
    cursorIndex = there;
    paint();
  }
}

export function draw() {
  paint();
}

// A desk wakes the ring by hovering and shuts it by leaving; a touch has no edge to leave by, so a tap off the ring is its way out.
export function wire() {
  const wrap = at("dial-wrap");
  wrap.addEventListener("pointerenter", (event) => {
    if (event.pointerType === "touch") return;
    clearTimeout(waking);
    if (!active) waking = setTimeout(wake, WAIT);
  });
  wrap.addEventListener("pointerleave", (event) => {
    if (event.pointerType === "touch") return;
    clearTimeout(waking);
    waking = 0;
    fold();
  });
  // Every press starts the flag over, whatever the pointer: a click only follows its own press, so a stale drag cannot swallow it.
  wrap.addEventListener("pointerdown", (event) => {
    dragged = false;
    if (event.pointerType !== "touch") return;
    if (!active) {
      wake();
      return;
    }
    grab = { id: event.pointerId, from: event.clientY, y: event.clientY, taken: false };
  });
  wrap.addEventListener("pointermove", (event) => {
    if (!grab || event.pointerId !== grab.id) return;
    // A finger this far meant to turn the ring, so what it does from here belongs to the ring alone: the capture keeps it coming past the box.
    if (!grab.taken) {
      if (Math.abs(event.clientY - grab.from) < SLIP) return;
      grab.taken = true;
      dragged = true;
      wrap.setPointerCapture(event.pointerId);
    }
    event.preventDefault();
    // A finger's way is the wheel's way: upward is later, though the axis it travels says the opposite.
    step(grab.y - event.clientY, PULL);
    grab.y = event.clientY;
  });
  const release = (event) => {
    if (!grab || event.pointerId !== grab.id) return;
    if (grab.taken && wrap.hasPointerCapture(event.pointerId)) {
      wrap.releasePointerCapture(event.pointerId);
    }
    grab = null;
    spun = 0;
    dir = 0;
  };
  wrap.addEventListener("pointerup", release);
  wrap.addEventListener("pointercancel", release);
  // A finger that dragged leaves no click behind on the word it started on: the turn was the whole of what it said.
  wrap.addEventListener(
    "click",
    (event) => {
      if (!dragged) return;
      event.stopPropagation();
      event.preventDefault();
    },
    true,
  );
  wrap.addEventListener("wheel", wheel, { passive: false });
  // Anything off the ring shuts it: on a screen with no pointer to leave with, a tap elsewhere is the way out.
  document.addEventListener(
    "pointerdown",
    (event) => {
      if (!active || wrap.contains(event.target)) return;
      fold();
    },
    true,
  );
  document.addEventListener("keydown", (event) => {
    if (event.key !== "Escape" || !active) return;
    fold();
  });
}
