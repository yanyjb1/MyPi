// The reader's place in the transcript: following the newest round, paging, and jumping to a round.

import { at } from "../../base/dom.js";
import { state } from "../../base/state.js";
import { blocks } from "../../base/read.js";
import { loaded } from "./rounds.js";
import { atEdge, center, grow, jumpRanges, prefetchNeeded } from "./window.js";
import { draw, moved, settle } from "./sheet.js";

// The round at the top of the reading column: the turns last laid down, in order, so a scroll never asks the
// transcript what it holds.
function head(chat) {
  const held = state.session;
  if (!held || !held.heads) return null;
  const top = chat.getBoundingClientRect().top + 8;
  for (const row of held.heads) {
    if (row.node.isConnected && row.node.getBoundingClientRect().bottom > top) {
      return row.turn;
    }
  }
  return null;
}

function checkPrefetch(held, isTop) {
  const needed = prefetchNeeded(held, isTop ? "top" : "bottom");
  if (needed.length) blocks(held.id, needed);
}

// The reader's place: pinned to the newest round, or a page that follows them towards either edge.
export function follow() {
  const chat = at("chat");
  let lastScrollTop = chat.scrollTop;
  chat.addEventListener("scroll", () => {
    const held = state.session;
    if (!held || moved()) return;
    const isUp = chat.scrollTop < lastScrollTop;
    lastScrollTop = chat.scrollTop;
    state.pinned = chat.scrollHeight - chat.scrollTop - chat.clientHeight < 48;
    const seen = head(chat);
    if (seen != null) held.anchor = seen;

    if (isUp && chat.scrollTop < 480) checkPrefetch(held, true);
    else if (
      !isUp && chat.scrollHeight - chat.scrollTop - chat.clientHeight < 480
    ) checkPrefetch(held, false);

    if (chat.scrollTop < 96 && atEdge(held, "top")) {
      grow(held, "top");
      draw();
      return;
    }
    // The newest page grows only for a reader who is already at the bottom, so a frame never pulls one back.
    if (state.pinned && atEdge(held, "bottom")) {
      grow(held, "bottom");
      draw();
    }
  });

  // A page step of the reader's own, held to one a moment so a held key does not run away.
  let lastPageTime = 0;
  window.addEventListener("keydown", (event) => {
    if (event.key !== "PageUp" && event.key !== "PageDown") return;
    event.preventDefault();
    const now = performance.now();
    if (now - lastPageTime < 50) return;
    lastPageTime = now;
    const held = state.session;
    if (!held) return;
    const isUp = event.key === "PageUp";
    checkPrefetch(held, isUp);
    const step = chat.clientHeight * 0.85;
    chat.scrollTop += isUp ? -step : step;
  });
}

export async function jump(turn) {
  const held = state.session;
  if (!held) return;
  const round = held.structure.find((one) => one.turn === turn);
  if (!round) return;
  state.pinned = false;
  held.anchor = turn;
  center(held, turn);
  held.view.delete(turn);

  const needed = jumpRanges(held, turn);
  if (!loaded(held, round)) (round.ranges || []).forEach((r) => needed.push(r));
  if (needed.length) await blocks(held.id, needed);

  draw();
  const node = at("sheet").querySelector('[data-turn="' + turn + '"]');
  if (node) {
    const top = at("chat").scrollTop + node.getBoundingClientRect().top -
      at("chat").getBoundingClientRect().top - 8;
    settle(at("chat"), top);
  }
}

export function latest() {
  const held = state.session;
  if (!held) return;
  held.span = null;
  state.pinned = true;
  draw();
  settle(at("chat"), at("chat").scrollHeight);
}
