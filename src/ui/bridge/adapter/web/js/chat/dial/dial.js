// The dial: five notches thirty degrees apart, the round in view always the one in the middle.

import { at, clear, el } from "../../base/dom.js";
import { state } from "../../base/state.js";
import { jump } from "../history/place.js";
import { label } from "./words.js";

// The arc a notch sits on, in the svg's own units: centre (99,100), radius 96, bulging left. Bearings run
// negative along the sweep, so the middle of the ring is -180: the round in view sits straight out from the hub.
// A larger bearing is nearer the top, so the list is laid out with the later rounds below: earlier rounds read
// upward on the ring's upper arm, later ones downward on its lower arm, exactly as a wheel over a list moves.
const CX = 99;
const CY = 100;
const R = 96;
const MID = -180;
const STEP = 30;
// A half ring holds seven bearings; the topmost and the bottommost are left empty, because a line there would
// have no room to say anything. Five it is, and the round in view keeps the middle one whatever its place in
// the list: the window slides under it, it never moves.
const SPAN = 2;

// A pointer that comes to rest on the ring wakes it; one that leaves folds it at once, so the ring never
// stays up behind a hand that has already gone.
const WAIT = 180;

let active = false;
let waking = 0;
let cursorIndex = -1;

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
  const tick = el("line", {
    class: "tick",
    x1,
    y1,
    x2,
    y2,
    "data-turn": String(round.turn),
    "vector-effect": "non-scaling-stroke",
  });
  tick.addEventListener("click", (event) => {
    event.stopPropagation();
    cursorIndex = -1;
    jump(round.turn);
  });
  return tick;
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
    items.push({ round, angle, offset: k, isSelected: (here + k) === here });
  }
  // The selected notch stays lit
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
  paint();
}

let dir = 0;

// Wheel wanders within dial cursor ONLY: content is NOT scrolled and full text blocks are NOT fetched.
function wheel(event) {
  if (!active) return;
  event.preventDefault();
  event.stopPropagation();
  const list = rounds();
  if (list.length < 2) return;
  dir = event.deltaY > 0 ? 1 : -1;
  const here = bearing();
  const there = Math.min(
    Math.max(here + dir, 0),
    list.length - 1,
  );
  if (there !== here) {
    cursorIndex = there;
    paint();
  }
}

export function draw() {
  paint();
}

// The ring has two states: five ticks to glance at, and a navigator that owns the wheel. One box holds the
// ticks and their words, so entering and leaving it is the whole of the hover.
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
  // A screen with no pointer keeps a tap as its only way in and out.
  wrap.addEventListener("pointerdown", (event) => {
    if (event.pointerType !== "touch") return;
    if (active) fold();
    else wake();
  });
  wrap.addEventListener("wheel", wheel, { passive: false });
  document.addEventListener("keydown", (event) => {
    if (event.key !== "Escape" || !active) return;
    fold();
  });
}
