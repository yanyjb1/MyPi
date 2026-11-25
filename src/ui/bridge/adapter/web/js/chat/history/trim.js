// What a session gives up when it holds more than its quota: the blocks farthest from the reader, seen longest ago.

import { MAX_SETTLED_BLOCKS, seen } from "./cap.js";

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
  if (held.blocks.size <= MAX_SETTLED_BLOCKS) return;
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
  let left = held.blocks.size - MAX_SETTLED_BLOCKS;
  for (const one of drop) {
    if (left <= 0) break;
    held.blocks.delete(one.id);
    held.view.delete(one.turn);
    left -= 1;
  }
}
