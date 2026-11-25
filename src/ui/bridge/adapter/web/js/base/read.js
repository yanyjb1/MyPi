// Everything the page asks the server for, and the one answer each becoming page state.

import { call } from "./wire.js";
import { announce, ensure, failed, state } from "./state.js";

// The session list, anchor rows left out: a session's anchor is not a session.
export async function refresh() {
  const before = fingerprint(state.rows);
  let changed = false;
  try {
    const answer = await call("sessions");
    const rows = (answer.rows || []).filter((row) => !row.anchor);
    changed = fingerprint(rows) !== before;
    state.rows = rows;
    if (state.rows.length && !state.active) {
      state.active = state.rows[0].id;
      changed = true;
    }
  } catch (e) {
    failed(e);
  }
  // A list nobody changed is not a page's worth of redraw: the rail polls this every few seconds.
  if (changed) announce();
}

// What a list of rows says, as one string to compare.
function fingerprint(rows) {
  return JSON.stringify(rows || []);
}

// One conversation's rows: its structure, the blocks it names, and the rungs it runs at.
export async function open(id) {
  const held = ensure(id);
  try {
    const [snapshot, meta] = await Promise.all([
      call("init", { session: id }),
      call("meta", { session: id }),
    ]);
    held.meta = meta;
    held.structure = snapshot.structure || [];
    held.blocks = new Map();
    held.view.clear();
    held.heads = null;
    held.span = null;
    for (const block of snapshot.blocks || []) held.blocks.set(block.id, block);
    held.todo = snapshot.todo || [];
    held.effort = snapshot.effort || "auto";
  } catch (e) {
    failed(e);
  }
  announce();
}

// Ranges asked for and not yet answered: two questions about the same blocks are one question.
const pendingRanges = new Set();

// Bodies for the ranges asked for: deduped in-flight, without redundant bill re-queries.
export async function blocks(id, ranges) {
  const held = ensure(id);
  const uncached = (ranges || []).filter(([lo, hi]) => {
    const key = id + ":" + lo + "-" + hi;
    if (pendingRanges.has(key)) return false;
    pendingRanges.add(key);
    return true;
  });
  if (!uncached.length) return;

  try {
    const answer = await call("blocks", { session: id, ranges: uncached });
    for (const block of answer.blocks || []) held.blocks.set(block.id, block);
    // A round whose own ranges meet a fetched one is laid again: it is found by sweeping the rounds once,
    // rather than once per range.
    for (const round of held.structure) {
      if (uncached.some(([lo, hi]) => meets(round.ranges, lo, hi))) {
        held.view.delete(round.turn);
      }
    }
    announce();
  } catch (e) {
    failed(e);
  } finally {
    for (const [lo, hi] of uncached) {
      pendingRanges.delete(id + ":" + lo + "-" + hi);
    }
  }
}

// Whether any of a round's own ranges meets the one just fetched.
function meets(ranges, lo, hi) {
  return (ranges || []).some(([start, end]) => start <= hi && lo <= end);
}

// A bill asked for while an earlier one is still on its way: the answer in flight settles both, since a lap's
// quote arrives far faster than the ledger takes to read.
const bills = new Map();

export async function bill(id) {
  const at = id || state.active;
  if (!at) return;
  const flying = bills.get(at);
  if (flying) return flying;
  const asked = call("usage", { session: at })
    .then((usage) => {
      ensure(at).usage = usage;
      announce("bill");
    })
    .catch(failed)
    .finally(() => bills.delete(at));
  bills.set(at, asked);
  return asked;
}

export async function todos(id) {
  const at = id || state.active;
  if (!at) return;
  try {
    const list = await call("todo", { session: at });
    ensure(at).todo = list || [];
    announce();
  } catch (e) {
    failed(e);
  }
}

export async function memos(id) {
  const at = id || state.active;
  if (!at) return;
  try {
    const answer = await call("memo", { session: at, op: "list" });
    ensure(at).memos = answer.memos || [];
    announce();
  } catch (e) {
    failed(e);
  }
}

export async function memoHistory(id, scope, name) {
  try {
    const answer = await call("memo", {
      session: id || state.active,
      op: "history",
      scope,
      name,
    });
    const held = ensure(state.active);
    held.read = { scope, name, versions: answer.versions || [] };
    announce();
  } catch (e) {
    failed(e);
  }
}

export async function kids() {
  const id = state.active;
  if (!id) return;
  try {
    const answer = await call("children", { session: id });
    ensure(id).children = answer.rows || [];
    announce();
  } catch (e) {
    failed(e);
  }
}

export async function pools(where) {
  try {
    state.pools = await call("pools", where ? { cwd: where } : {});
    announce();
  } catch (e) {
    failed(e);
  }
}
