// The keys: what the composer claims, what the popover claims first, and what leaves the box.

import { at } from "../../base/dom.js";
import { empty, value } from "./say.js";
import { closePanel, state } from "../../base/state.js";
import { summon } from "./summon.js";
import { send } from "./talk.js";
import { isDraft, speak } from "../../rail/draft.js";
import { accept, close, complete, isOpen, move as movePick } from "./path.js";
import { interrupt } from "../../base/write.js";

// One line: a command the page itself runs, or a message. A draft becomes a session on its first message.
async function submit() {
  const text = value(at("say"));
  if (!text.trim()) return;
  if (summon(text)) {
    empty(at("say"));
    return;
  }
  empty(at("say"));
  close();
  if (isDraft()) await speak(text);
  else send(text);
}

// A chip is one token for the cursor and one token for Backspace.
function wholeChip(event) {
  const picked = window.getSelection();
  if (!picked || !picked.rangeCount) return false;
  const range = picked.getRangeAt(0);
  if (!range.collapsed) return false;
  let node = range.startContainer;
  if (node.nodeType !== Node.TEXT_NODE) {
    node = node.childNodes[range.startOffset] || null;
  }
  if (
    node && node.nodeType !== Node.TEXT_NODE && node.classList &&
    node.classList.contains("chip-token")
  ) {
    event.preventDefault();
    node.remove();
    return true;
  }
  return false;
}

// The arrows, Tab and Enter belong to the popover while it stands; everything else falls through.
function claimed(event) {
  if (!isOpen()) return false;
  if (event.key === "ArrowDown") {
    event.preventDefault();
    movePick(1);
    return true;
  }
  if (event.key === "ArrowUp") {
    event.preventDefault();
    movePick(-1);
    return true;
  }
  if (event.key === "Tab" || event.key === "Enter") {
    event.preventDefault();
    accept();
    return true;
  }
  if (event.key === "Escape") {
    event.preventDefault();
    close();
    return true;
  }
  return false;
}

export function keys() {
  const box = at("say");

  box.addEventListener("keydown", (event) => {
    if (claimed(event)) return;
    if (event.key === "Backspace" && wholeChip(event)) return;
    if (event.key === "Delete" && wholeChip(event)) return;
    if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      submit();
    }
  });

  box.addEventListener("input", () => complete(at("say")));
  box.addEventListener("blur", () => setTimeout(close, 120));

  // The directory field completes its own word: Tab and Enter pick from the same popover.
  at("cwd").addEventListener("blur", () => setTimeout(close, 120));

  window.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && state.panel.open) {
      event.preventDefault();
      closePanel();
    }
    if (event.key === "." && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      interrupt();
    }
  });
}

export { submit };
