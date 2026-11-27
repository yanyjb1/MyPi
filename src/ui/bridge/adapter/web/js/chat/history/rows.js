// The blocks of one round, oldest first, with each tool result paired to its request.

function roundRows(rows) {
  const requests = new Map();
  for (const block of rows) {
    if (block.kind === "tool_request" && block.call_id) {
      requests.set(block.call_id, block);
    }
  }
  return rows.map((block) => {
    if (
      block.kind === "tool_result" && block.call_id &&
      requests.has(block.call_id)
    ) {
      return { block, paired: requests.get(block.call_id) };
    }
    return { block, paired: null };
  });
}

// One round's blocks: the ranges say which ids it owns, so each is asked for by its own number rather than
// found by sweeping every block in hand.
export function blocksOf(held, round) {
  const found = [];
  for (const [lo, hi] of round.ranges || []) {
    for (let id = lo; id <= hi; id += 1) {
      const block = held.blocks.get(id);
      if (block) found.push(block);
    }
  }
  return roundRows(found);
}
