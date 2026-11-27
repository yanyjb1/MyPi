// One landed round of the transcript: what its blocks say, in the order they came.

import { el } from "../../base/dom.js";
import { body } from "./mark.js";
import { parsed, reasoning, tool, wake } from "./card.js";
import { blocksOf } from "./rows.js";

// What the reader said, split into the prefix, the words and the suffix they carry.
function you(block) {
  const kids = [];
  if (block.prefix) {
    kids.push(el("span", { class: "wrap", text: block.prefix }));
  }
  kids.push(el("span", { text: block.content || block.text || "" }));
  if (block.suffix) {
    kids.push(el("span", { class: "wrap", text: block.suffix }));
  }
  return el("div", { class: "you" }, kids);
}

// One landed block: a request with its result, or one of the message bodies.
function blockView(block, paired) {
  switch (block.kind) {
    case "user":
      return you(block);
    case "wake":
      return wake(block.text || block.content);
    case "assistant":
      return el("div", {
        class: "md",
        html: body(block.content || block.text),
      });
    case "reasoning":
      return reasoning(block.content || block.text);
    case "todo":
      return null;
    case "tool_request": {
      const items = paired && Array.isArray(paired.ok) ? paired.ok : [];
      const failed = items.some((item) => item && item.ok === false);
      return tool(
        block.name,
        parsed(block.args),
        block.intent,
        paired ? (failed ? "failed" : "done") : "running",
        paired ? paired.result : "",
        paired ? paired.duration_ms : 0,
        "",
      );
    }
    case "tool_result": {
      if (paired) return null;
      const items = Array.isArray(block.ok) ? block.ok : [];
      const failed = items.some((item) => item && item.ok === false);
      return tool(
        block.name,
        {},
        "",
        failed ? "failed" : "done",
        block.result,
        block.duration_ms,
        "",
      );
    }
    default:
      return el("div", {
        class: "md",
        html: body(String(block.content || block.text || "")),
      });
  }
}

// Whether every block a round owns has arrived: a round without them is nothing to draw.
export function loaded(held, round) {
  for (const [lo, hi] of round.ranges || []) {
    for (let id = lo; id <= hi; id += 1) if (!held.blocks.has(id)) return false;
  }
  return true;
}

// The user's own block in a round, once its blocks are here.
function youSaid(held, round) {
  for (const [lo, hi] of round.ranges || []) {
    for (let id = lo; id <= hi; id += 1) {
      const block = held.blocks.get(id);
      if (block && block.kind === "user") return block;
    }
  }
  return null;
}

// The user's own words in a round, once its blocks are here: what a jump list says.
export function said(held, round) {
  const block = youSaid(held, round);
  if (!block) return "";
  return String(block.prefix || "") + String(block.content || block.text || "");
}

// The words alone, without the wrapper around them: exactly what an echo of another front end says.
export function spoken(held, round) {
  const block = youSaid(held, round);
  if (!block) return "";
  return String(block.content || block.text || "");
}

// One round as it stands: a compaction says so, then every block it owns, oldest first.
export function turnNode(held, round) {
  const turn = el("section", {
    class: "turn",
    "data-turn": String(round.turn),
  });
  if (round.compact) {
    turn.append(
      el(
        "div",
        { class: "who" },
        el("span", { class: "accent", text: "compaction" }),
      ),
    );
  }
  for (const { block, paired } of blocksOf(held, round)) {
    const view = blockView(block, paired);
    if (view) turn.append(view);
  }
  return turn;
}
