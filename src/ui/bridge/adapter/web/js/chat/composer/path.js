// The popover over the composer: the candidates one word offers, and what picking one writes back.

import { at, clear, el } from "../../base/dom.js";
import { replace, token, word } from "./say.js";
import { directory, offer } from "./offer.js";

const menu = () => at("complete-menu");

let items = [];
let picked = 0;
let spot = null;
let target = null;
let timer = 0;

export function isOpen() {
  return !menu().hidden;
}

export function close() {
  menu().hidden = true;
  items = [];
  picked = 0;
  spot = null;
  target = null;
}

// The word the cursor sits in, of the field that has it: the composer's own, or the directory's.
function here(field) {
  if (!field || field.id !== "cwd") return word(at("say"));
  const box = at("cwd");
  const edge = box.selectionStart == null
    ? box.value.length
    : box.selectionStart;
  return { text: box.value.slice(0, edge), at: 0 };
}

export async function refresh(field) {
  clearTimeout(timer);
  target = field && field.id === "cwd" ? "cwd" : "say";
  const found = here(field);
  if (!found) {
    close();
    return;
  }
  let shown;
  try {
    shown = target === "cwd" ? await directory(found.text) : await offer();
  } catch (e) {
    // A directory this machine will not list is said where the list would have stood.
    spot = found;
    items = [];
    picked = 0;
    render(e instanceof Error ? e.message : String(e));
    return;
  }
  // A word one candidate already spells is done: the popup closes so the next Enter is the line's own.
  if (!shown.length || leaf(shown, found.text)) {
    close();
    return;
  }
  items = shown;
  spot = found;
  picked = 0;
  render();
}

// Whether the word is finished: one candidate spells it, or a directory's own `/` ends it.
function leaf(shown, typed) {
  const want = typed.replace(/^\//, "").trimEnd();
  return shown.some((item) =>
    item.insert.trimEnd().replace(/^\//, "") === want
  );
}

// The list is drawn as it stands; the highlight is the only thing a key moves.
function render(reason) {
  const box = clear(menu());
  if (reason) {
    box.append(
      el("div", { class: "item" }, el("span", { class: "dir", text: reason })),
    );
    menu().hidden = false;
    return;
  }
  items.forEach((item, index) => {
    box.append(
      el(
        "div",
        { class: "item", "data-on": index === picked ? "1" : "0" },
        el("span", { class: item.dir ? "dir" : null, text: item.name }),
        item.detail ? el("span", { class: "dir", text: item.detail }) : null,
      ),
    );
  });
  menu().hidden = false;
}

export function move(delta) {
  if (!items.length) return;
  picked = Math.min(Math.max(picked + delta, 0), items.length - 1);
  render();
}

// The picked item lands in the field, replacing the word the cursor sits in.
export function accept() {
  const item = items[picked];
  if (!item) return;
  if (target === "cwd") {
    const box = at("cwd");
    box.value = item.insert;
    box.setSelectionRange(item.insert.length, item.insert.length);
    close();
    if (item.dir) refresh(box);
    return;
  }
  const box = at("say");
  // A file is one chip; a directory, a command and a bare name are text.
  if (item.kind === "path" && !item.dir) {
    replace(box, spot, token(item.insert, item.name), " ");
  } else {
    replace(box, spot, document.createTextNode(item.insert), " ");
  }
  close();
  refresh(box);
}

// Typing again asks again, but not on every keystroke.
export function complete(field) {
  clearTimeout(timer);
  timer = setTimeout(() => refresh(field), 120);
}

export { replace };
