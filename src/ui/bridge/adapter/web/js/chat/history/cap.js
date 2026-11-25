// How many rows are held: the window a glance covers, the margin read ahead of it, and the quota a session keeps.

export const SETTLED_WINDOW_BLOCKS = 64;
export const MARGIN_BLOCKS = 32;
export const MAX_SETTLED_BLOCKS = 256;
export const JUMP_RADIUS = 8;

// The blocks one round names, counted rather than measured: a round of no ranges still takes a row.
export function roundBlocks(round) {
  let count = 0;
  for (const [s, e] of (round && round.ranges) || []) count += e - s + 1;
  return count || 1;
}

// The turns the reader has put on screen, newest last.
const touched = new Map();

export function touch(turn) {
  touched.set(turn, Date.now());
}

// When a turn was last on screen; a turn nobody has seen ranks as the oldest.
export function seen(turn) {
  return touched.get(turn) || 0;
}
