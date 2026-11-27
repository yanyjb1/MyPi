// The transcript: one page of rounds, the live tail, and the one refusal. A page is laid whole, and a stream frame
// then touches the tail alone.

import { at, build, clear, text } from "../../core/template.js";
import { dismiss, state } from "../../core/store.js";
import { blocks } from "../../core/api.js";
import { body } from "../../render/markdown.js";
import { card } from "../../render/tools.js";
import { loaded, slice, trim, watched } from "./window.js";
import { liveTurn } from "./live.js";
import { noteLine } from "./note.js";

let trimTimer = 0;

// Set while the page moves itself, so its own scroll write is never read back as the reader scrolling.
let moving = false;

export const moved = () => moving;

export function settle(chat, where) {
  moving = true;
  chat.scrollTop = where;
  requestAnimationFrame(() => {
    moving = false;
  });
}

// The reader's own line: the prefix, the words, and the suffix a wrapper put around them.
function you(block) {
  const node = build("tpl-you");
  text(node, "prefix", block.prefix);
  text(node, "said", block.content || block.text);
  text(node, "suffix", block.suffix);
  return node;
}

// A block whose body is markdown, laid as HTML wherever it stands.
function prose(source) {
  const node = build("tpl-prose");
  node.innerHTML = body(source || "");
  return node;
}

// A thinking block, rendered in muted prose and folded to one line.
function thinking(source) {
  const node = build("tpl-think");
  node.querySelector('[data-ref="body"]').innerHTML = body(source || "");
  if (state.streaming) node.open = true;
  return node;
}

// One landed block: a request with its result, or one of the message bodies.
function blockView(block, paired) {
  switch (block.kind) {
    case "user":
      return you(block);
    case "wake":
      return noteLine("notice", "wake", block.text || block.content);
    case "assistant":
      return prose(block.content || block.text);
    case "reasoning":
      return thinking(block.content || block.text);
    case "todo":
      return null;
    case "tool_request": {
      const items = paired && Array.isArray(paired.ok) ? paired.ok : [];
      const failed = items.some((item) => item && item.ok === false);
      return card(
        block.name,
        block.args,
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
      return card(
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
      return prose(String(block.content || block.text || ""));
  }
}

// The blocks of one round, oldest first, with each tool result paired to its request.
function blocksOf(held, round) {
  const found = [];
  for (const [lo, hi] of round.ranges || []) {
    for (let id = lo; id <= hi; id += 1) {
      const block = held.blocks.get(id);
      if (block) found.push(block);
    }
  }
  const requests = new Map();
  for (const block of found) {
    if (block.kind === "tool_request" && block.call_id) {
      requests.set(block.call_id, block);
    }
  }
  return found.map((block) => {
    if (
      block.kind === "tool_result" && block.call_id &&
      requests.has(block.call_id)
    ) {
      return { block, paired: requests.get(block.call_id) };
    }
    return { block, paired: null };
  });
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
  const turn = build("tpl-turn");
  turn.dataset.turn = String(round.turn);
  if (round.compact) turn.querySelector('[data-ref="who"]').hidden = false;
  for (const { block, paired } of blocksOf(held, round)) {
    const view = blockView(block, paired);
    if (view) turn.append(view);
  }
  return turn;
}

// A round whose body has not been read: one quiet line, and a click or a glance at it reads it.
function gap(held, round) {
  const node = build("tpl-gap");
  node.dataset.turn = String(round.turn);
  const asked = async () => {
    if (node.dataset.asked === "1") return;
    node.dataset.asked = "1";
    await blocks(held.id, round.ranges || []);
    held.view.delete(round.turn);
    draw();
  };
  node.addEventListener("click", asked);
  watched.observe(node, asked);
  return node;
}

// Whether the live tail already says this round. The server writes a round's opening block the moment it starts, so
// the landed copy below the tail would say that same user line a second time.
function saying(held, round) {
  if (round.turn === held.liveTurn && held.live.length) return true;
  const opening = held.live.find((item) => item.type === "user");
  if (!opening || !held.busy) return false;
  const newest = held.structure[held.structure.length - 1];
  return newest === round && spoken(held, round) === opening.text;
}

// One page: the rounds of the span from the node cache, the live tail under them, the one refusal.
export function draw() {
  const chat = at("chat");
  const held = state.session;
  if (!held) return;
  const kept = chat.scrollHeight;
  const top = chat.scrollTop;
  const pinned = state.pinned;
  const sheet = clear(at("sheet"));
  // Every node this page lays down is new: the live round's own nodes go with the page it was drawn on.
  held.liveNode = null;
  if (!held.structure.length && !held.live.length) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = "this session is empty — say something";
    sheet.append(empty);
  }
  const rounds = slice(held).filter((round) => !saying(held, round));
  for (const round of rounds) {
    let node = held.view.get(round.turn);
    if (!node) {
      node = loaded(held, round) ? turnNode(held, round) : gap(held, round);
      held.view.set(round.turn, node);
    }
    sheet.append(node);
  }
  // What the reading column holds, for the next scroll to read without asking the page.
  held.heads = rounds.map((round) => ({
    turn: round.turn,
    node: held.view.get(round.turn),
  }));

  if (held.live.length) {
    // This page builds every node again, so each live piece builds its own again with it.
    for (const item of held.live) item.view = null;
    sheet.append(liveTurn(held));
  }
  held.refusalNode = null;
  if (state.refusal) {
    const line = noteLine("error", "refused", state.refusal);
    const close = document.createElement("button");
    close.className = "icon";
    close.textContent = "×";
    close.addEventListener("click", dismiss);
    line.append(close);
    sheet.append(line);
    held.refusalNode = line;
  }
  scheduleTrim(held);
  if (pinned) settle(chat, chat.scrollHeight);
  else if (chat.scrollHeight > kept) {
    settle(chat, top + (chat.scrollHeight - kept));
  }
}

function scheduleTrim(held) {
  clearTimeout(trimTimer);
  trimTimer = setTimeout(() => trim(held), 250);
}

// The live tail alone: the only part a stream frame touches, and inside it only the piece that grew.
export function tail() {
  const held = state.session;
  if (!held) return;
  const sheet = at("sheet");
  if (!held.live.length) {
    if (held.liveNode && held.liveNode.parentNode === sheet) {
      held.liveNode.remove();
    }
    held.liveNode = null;
    return;
  }
  const node = liveTurn(held);
  // The live round stands above the one refusal, wherever the refusal is: a refusal that arrived over a running
  // round does not put the round under it.
  if (node.parentNode !== sheet) {
    const anchor = held.refusalNode;
    sheet.insertBefore(
      node,
      anchor && anchor.parentNode === sheet ? anchor : null,
    );
  }
  if (state.pinned) settle(at("chat"), at("chat").scrollHeight);
}