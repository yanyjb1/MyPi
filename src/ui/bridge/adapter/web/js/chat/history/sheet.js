// The transcript: one page of rounds, the live tail, and the reader's own place in it.

import { at, clear, el } from "../../base/dom.js";
import { dismiss, state } from "../../base/state.js";
import { noteLine } from "./card.js";
import { blocks } from "../../base/read.js";
import { loaded, spoken, turnNode } from "./rounds.js";
import { liveTurn } from "./liveturn.js";
import { slice, trim } from "./window.js";
import { watched } from "./watched.js";

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

function gap(held, round) {
  const node = el("div", {
    class: "turn gap",
    "data-turn": String(round.turn),
  });
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

// Whether the live tail already says this round. The server writes a round's opening block the moment
// it starts, so the landed copy below the tail would say that same user line a second time.
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
    sheet.append(
      el("div", {
        class: "empty",
        text: "this session is empty — say something",
      }),
    );
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
    line.append(el("button", { class: "icon", text: "×", onclick: dismiss }));
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
