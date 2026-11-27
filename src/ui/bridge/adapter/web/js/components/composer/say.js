// The composer's text: what the box holds, and the word the cursor sits in. A file is one chip, never edited, and
// its own text names the path it stands for.

import { at } from "../../core/template.js";

export function token(value, label) {
  const chip = document.createElement("span");
  chip.className = "chip-token";
  chip.setAttribute("contenteditable", "false");
  chip.dataset.kind = "file";
  chip.dataset.value = value;
  chip.textContent = label;
  return chip;
}

// The box's whole text, chips spelled as their own path.
export function value(box) {
  let out = "";
  const walk = (node) => {
    for (const kid of node.childNodes) {
      if (kid.nodeType === Node.TEXT_NODE) out += kid.textContent;
      else if (kid.classList && kid.classList.contains("chip-token")) {
        out += kid.dataset.value || kid.textContent;
      } else if (kid.nodeName === "BR") out += "\n";
      else if (kid.nodeName === "DIV") {
        if (out && !out.endsWith("\n")) out += "\n";
        walk(kid);
      } else walk(kid);
    }
  };
  walk(box);
  return out;
}

export function empty(box) {
  box.textContent = "";
}

// Everything before the cursor, as one string: the command line and the word both read it.
export function before(box) {
  const picked = window.getSelection();
  if (!picked || !picked.rangeCount) return "";
  const range = picked.getRangeAt(0);
  if (!box.contains(range.startContainer)) return "";
  const whole = document.createRange();
  whole.selectNodeContents(box);
  whole.setEnd(range.startContainer, range.startOffset);
  return whole.toString();
}

// The word the cursor sits in, as typed, and where it starts inside the box's text.
export function word(box) {
  const said = before(box);
  const start = /\S*$/.exec(said)[0].length;
  return { text: said.slice(said.length - start), at: said.length - start };
}

// Replace the word the cursor sits in with one node: a chip for a file, plain text for anything else.
export function replace(box, span, node, tail) {
  const picked = window.getSelection();
  if (!picked || !picked.rangeCount) return;
  const range = picked.getRangeAt(0);
  const said = before(box);
  if (said.length < span.at + span.text.length) return;
  const tailRange = document.createRange();
  tailRange.selectNodeContents(box);
  tailRange.setEnd(range.startContainer, range.startOffset);
  moveBack(tailRange, span.text.length);
  const gone = document.createRange();
  gone.setStart(tailRange.endContainer, tailRange.endOffset);
  gone.setEnd(range.startContainer, range.startOffset);
  gone.deleteContents();
  gone.insertNode(node);
  const spot = document.createRange();
  if (tail) {
    const after = document.createTextNode(tail);
    node.parentNode.insertBefore(after, node.nextSibling);
    spot.setStartAfter(after);
  } else {
    spot.setStartAfter(node);
  }
  spot.collapse(true);
  picked.removeAllRanges();
  picked.addRange(spot);
}

// Walk a range's end back by a number of characters inside the same node run.
function moveBack(range, count) {
  let left = count;
  const node = range.endContainer;
  if (node.nodeType === Node.TEXT_NODE) {
    range.setEnd(node, Math.max(0, range.endOffset - left));
    return;
  }
  let at = range.endOffset;
  while (left > 0 && at > 0) {
    at -= 1;
    const kid = node.childNodes[at];
    const size = kid.textContent.length;
    if (size <= left) {
      left -= size;
    } else {
      range.setEnd(kid, size - left);
      return;
    }
  }
  range.setEnd(node, at);
}

export { at };
