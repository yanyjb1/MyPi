// The box itself and the directory field under it: a path that becomes editable on a double click, and what the
// page says once the directory has moved.

import { at } from "../../core/template.js";
import { address, announce, slot, state } from "../../core/store.js";
import { entry, fallbackModel, listing, move, pools } from "../../core/api.js";
import { accept, close, complete, isOpen, move as movePick } from "./path.js";
import { showCwdFeedback, where } from "./mods.js";

function edit() {
  const field = at("cwd");
  if (!field.readOnly) return;
  field.readOnly = false;
  field.dataset.moved = "0";
  field.focus();
  field.select();
  complete(field);
}

function settle() {
  const field = at("cwd");
  if (field.readOnly) return;
  field.readOnly = true;
  close();
  const path = where();
  field.value = path;
  field.scrollLeft = field.scrollWidth;
  field.title = path;
}

async function commitCwd() {
  const path = at("cwd").value.trim();
  if (state.session) {
    const ok = await move(path);
    showCwdFeedback(Boolean(ok));
  } else {
    slot().cwd = path;
    showCwdFeedback(true);
    announce();
  }
  settle();
  at("say").focus();
}

function handleCwdKey(event) {
  if (isOpen()) {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      movePick(event.key === "ArrowDown" ? 1 : -1);
      return;
    }
    if (event.key === "Tab") {
      event.preventDefault();
      accept();
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      close();
      return;
    }
  }
  if (event.key === "Escape") {
    event.preventDefault();
    settle();
    at("say").focus();
  } else if (event.key === "Enter") {
    event.preventDefault();
    commitCwd();
  }
}

export function wire() {
  at("cwd").addEventListener("dblclick", edit);
  at("cwd").addEventListener("blur", settle);
  at("cwd").addEventListener("keydown", handleCwdKey);
  at("cwd").addEventListener("input", () => complete(at("cwd")));
  // The field keeps the tail of its path in view itself, so a window grab shows which directory it is in.
  at("cwd").addEventListener(
    "scroll",
    () => {
      const field = at("cwd");
      if (field.readOnly) field.scrollLeft = field.scrollWidth;
    },
    { passive: true },
  );
  // Hold the field's view on the tail of its path: the last folders say where a session stands.
  new ResizeObserver(() => {
    const field = at("cwd");
    if (!field.readOnly) return;
    field.title = field.value || "";
    field.scrollLeft = field.scrollWidth;
  }).observe(at("cwd"));
}

// The first draw's own reading: where the page stands, the pools it picks from, and the model it would open with.
export async function boot() {
  slot();
  if (!state.draft.cwd) {
    try {
      const listed = await listing();
      if (listed.dir) state.draft.cwd = listed.dir;
    } catch {}
  }
  await pools();
  const picked = address() || (await fallbackModel());
  await entry(picked);
  if (!state.draft.model) state.draft.model = picked;
  announce();
}
