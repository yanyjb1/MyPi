// The window a session holds: which rounds are on screen, how the edge reaches for more, and how a jump lands.

import {
  JUMP_RADIUS,
  MARGIN_BLOCKS,
  roundBlocks,
  SETTLED_WINDOW_BLOCKS,
  touch,
} from "./cap.js";

export { trim } from "./trim.js";

export function tail(held) {
  const last = held.structure.length - 1;
  if (last < 0) {
    held.span = [0, -1];
    return held.span;
  }
  let blocks = 0;
  let lo = last;
  for (let i = last; i >= 0 && blocks < SETTLED_WINDOW_BLOCKS; i -= 1) {
    blocks += roundBlocks(held.structure[i]);
    lo = i;
  }
  held.span = [lo, last];
  return held.span;
}

export function clamp(held) {
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

export function indexOf(held, turn) {
  return held.structure.findIndex((round) => round.turn === turn);
}

// The window around one turn: as many rounds either side as the glance covers.
export function center(held, turn) {
  const at = indexOf(held, turn);
  if (at < 0) return clamp(held);
  const last = held.structure.length - 1;
  let lo = at;
  let hi = at;
  let blocks = roundBlocks(held.structure[at]);
  while (blocks < SETTLED_WINDOW_BLOCKS && (lo > 0 || hi < last)) {
    if (lo > 0) {
      lo -= 1;
      blocks += roundBlocks(held.structure[lo]);
    }
    if (hi < last && blocks < SETTLED_WINDOW_BLOCKS) {
      hi += 1;
      blocks += roundBlocks(held.structure[hi]);
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
    let blocks = 0;
    let newHi = newLo;
    for (let i = newLo; i <= last; i += 1) {
      blocks += roundBlocks(held.structure[i]);
      newHi = i;
      if (blocks >= SETTLED_WINDOW_BLOCKS && i >= lo) break;
    }
    held.span = [newLo, Math.min(last, Math.max(newHi, lo))];
  } else {
    const newHi = Math.min(last, hi + 1);
    let blocks = 0;
    let newLo = newHi;
    for (let i = newHi; i >= 0; i -= 1) {
      blocks += roundBlocks(held.structure[i]);
      newLo = i;
      if (blocks >= SETTLED_WINDOW_BLOCKS && i <= hi) break;
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
    if (contiguous >= MARGIN_BLOCKS) break;
    const ranges = held.structure[i].ranges || [];
    const order = top ? [...ranges].reverse() : ranges;
    for (const [start, end] of order) {
      if (contiguous >= MARGIN_BLOCKS) break;
      const bLo = top ? end : start;
      const bHi = top ? start : end;
      let missing = false;
      for (let b = bLo; top ? b >= bHi : b <= bHi; b += top ? -1 : 1) {
        if (contiguous >= MARGIN_BLOCKS) break;
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
  const lo = Math.max(0, centre - JUMP_RADIUS);
  const hi = centre + JUMP_RADIUS;
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
