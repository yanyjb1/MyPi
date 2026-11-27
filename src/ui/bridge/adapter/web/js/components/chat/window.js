// The window a session holds: which rounds are on screen, how the edge reaches for more, and what a session gives up
// when it holds too much. Nothing here draws.

import { at } from "../../core/template.js";

// How many rows are held: the window a glance covers, the margin read ahead of it, and the quota a session keeps.
const WINDOW = 64;
const MARGIN = 32;
const QUOTA = 256;
const RADIUS = 8;

// The blocks one round names, counted rather than measured: a round of no ranges still takes a row.
function weigh(round) {
  let count = 0;
  for (const [s, e] of (round && round.ranges) || []) count += e - s + 1;
  return count || 1;
}

// The turns the reader has put on screen, newest last, and when each was last there.
const touched = new Map();

function touch(turn) {
  touched.set(turn, Date.now());
}

// When a turn was last on screen; a turn nobody has seen ranks as the oldest.
function seen(turn) {
  return touched.get(turn) || 0;
}

function tail(held) {
  const last = held.structure.length - 1;
  if (last < 0) {
    held.span = [0, -1];
    return held.span;
  }
  let count = 0;
  let lo = last;
  for (let i = last; i >= 0 && count < WINDOW; i -= 1) {
    count += weigh(held.structure[i]);
    lo = i;
  }
  held.span = [lo, last];
  return held.span;
}

function clamp(held) {
  const last = held.structure.length - 1;
  if (!held.span) return tail(held);
  const [lo, hi] = held.span;
  held.span = [
    Math.max(0, Math.min(lo, last)),
    Math.max(0, Math.min(hi, last)),
  ];
  return held.span[1] < held.span[0] ? tail(held) : held.span;
}

export function slice(held) {
  const [lo, hi] = clamp(held);
  const rounds = held.structure.slice(lo, hi + 1);
  for (const round of rounds) touch(round.turn);
  return rounds;
}

function indexOf(held, turn) {
  return held.structure.findIndex((round) => round.turn === turn);
}

// The window around one turn: as many rounds either side as the glance covers.
export function center(held, turn) {
  const at = indexOf(held, turn);
  if (at < 0) return clamp(held);
  const last = held.structure.length - 1;
  let lo = at;
  let hi = at;
  let count = weigh(held.structure[at]);
  while (count < WINDOW && (lo > 0 || hi < last)) {
    if (lo > 0) {
      lo -= 1;
      count += weigh(held.structure[lo]);
    }
    if (hi < last && count < WINDOW) {
      hi += 1;
      count += weigh(held.structure[hi]);
    }
  }
  held.span = [lo, hi];
  touch(turn);
  return held.span;
}

export function atEdge(held, side) {
  const [lo, hi] = clamp(held);
  return side === "top" ? lo > 0 : hi < held.structure.length - 1;
}

// One side reaches one round further, then takes back a whole window's worth from that side.
export function grow(held, side) {
  const [lo, hi] = clamp(held);
  const last = held.structure.length - 1;
  if (side === "top") {
    const newLo = Math.max(0, lo - 1);
    let count = 0;
    let newHi = newLo;
    for (let i = newLo; i <= last; i += 1) {
      count += weigh(held.structure[i]);
      newHi = i;
      if (count >= WINDOW && i >= lo) break;
    }
    held.span = [newLo, Math.min(last, Math.max(newHi, lo))];
  } else {
    const newHi = Math.min(last, hi + 1);
    let count = 0;
    let newLo = newHi;
    for (let i = newHi; i >= 0; i -= 1) {
      count += weigh(held.structure[i]);
      newLo = i;
      if (count >= WINDOW && i <= hi) break;
    }
    held.span = [Math.max(0, Math.min(newLo, hi)), newHi];
  }
  return held.span;
}

// The margin read ahead of the window, walked outward from its edge: the rounds whose blocks are not in hand.
export function prefetchNeeded(held, side) {
  const [lo, hi] = clamp(held);
  const needed = [];
  let contiguous = 0;
  const top = side === "top";
  const from = top ? lo - 1 : hi + 1;
  const to = top ? 0 : held.structure.length - 1;
  const step = top ? -1 : 1;
  for (let i = from; top ? i >= to : i <= to; i += step) {
    if (contiguous >= MARGIN) break;
    const ranges = held.structure[i].ranges || [];
    const order = top ? [...ranges].reverse() : ranges;
    for (const [start, end] of order) {
      if (contiguous >= MARGIN) break;
      const bLo = top ? end : start;
      const bHi = top ? start : end;
      let missing = false;
      for (let b = bLo; top ? b >= bHi : b <= bHi; b += top ? -1 : 1) {
        if (contiguous >= MARGIN) break;
        if (held.blocks.has(b)) contiguous += 1;
        else missing = true;
      }
      if (missing) needed.push([start, end]);
    }
  }
  return needed;
}

// The rows around a turn a jump lands on: a radius either side, and nothing a round does not already name.
export function jumpRanges(held, turn) {
  const at = indexOf(held, turn);
  if (at < 0) return [];
  const first = (held.structure[at].ranges || [])[0];
  const centre = first ? first[0] : 0;
  const lo = Math.max(0, centre - RADIUS);
  const hi = centre + RADIUS;
  const needed = [];
  for (
    let i = Math.max(0, at - 4);
    i <= Math.min(held.structure.length - 1, at + 4);
    i += 1
  ) {
    for (const [start, end] of held.structure[i].ranges || []) {
      if (start > hi || end < lo) continue;
      for (let b = Math.max(start, lo); b <= Math.min(end, hi); b += 1) {
        if (held.blocks.has(b)) continue;
        needed.push([start, end]);
        break;
      }
    }
  }
  return needed;
}

// The blocks a round's ranges name, in order.
function ids(ranges) {
  const out = [];
  for (const [start, end] of ranges || []) {
    for (let b = start; b <= end; b += 1) out.push(b);
  }
  return out;
}

// Every block the window and the margin beside it keep: nothing here is a candidate.
function spared(structure, lo, hi) {
  const kept = new Set();
  for (
    let i = Math.max(0, lo - 1);
    i <= Math.min(structure.length - 1, hi + 1);
    i += 1
  ) {
    for (const b of ids(structure[i].ranges)) kept.add(b);
  }
  return kept;
}

// Evict settled blocks past the quota, the ones furthest from the reader's middle first.
export function trim(held) {
  if (held.blocks.size <= QUOTA) return;
  // A session with no window has nothing on screen to keep, and the quota can wait for the one that lands.
  if (!held.span) return;
  const [lo, hi] = held.span;
  const kept = spared(held.structure, lo, hi);
  const middle = held.structure[Math.floor((lo + hi) / 2)];
  const first = (middle && middle.ranges && middle.ranges[0]) || [];
  const centre = first[0] || 0;
  const drop = [];
  for (const round of held.structure) {
    const time = seen(round.turn);
    for (const b of ids(round.ranges)) {
      if (kept.has(b) || !held.blocks.has(b)) continue;
      drop.push({ id: b, time, dist: Math.abs(b - centre), turn: round.turn });
    }
  }
  drop.sort((a, b) => a.time - b.time || b.dist - a.dist);
  let left = held.blocks.size - QUOTA;
  for (const one of drop) {
    if (left <= 0) break;
    held.blocks.delete(one.id);
    held.view.delete(one.turn);
    left -= 1;
  }
}

// Whether every block a round owns has arrived: a round without them is nothing to draw.
export function loaded(held, round) {
  for (const [lo, hi] of round.ranges || []) {
    for (let id = lo; id <= hi; id += 1) if (!held.blocks.has(id)) return false;
  }
  return true;
}

// The gaps a page leaves for rounds it has not read: each asks for its own ranges the moment it is near.
export const watched = {
  seen: new Map(),
  observer: null,
  observe(node, act) {
    if (!this.observer) {
      this.observer = new IntersectionObserver((rows) => {
        for (const row of rows) {
          if (!row.isIntersecting) continue;
          const run = this.seen.get(row.target);
          if (run) {
            this.seen.delete(row.target);
            this.observer.unobserve(row.target);
            run();
          }
        }
      }, { root: at("chat"), rootMargin: "120px" });
    }
    this.seen.set(node, act);
    this.observer.observe(node);
  },
};