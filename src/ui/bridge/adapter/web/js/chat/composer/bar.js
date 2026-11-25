// The dressing under the box: the directory, the branch, the bill, and the two picks.

import { at, clear, el } from "../../base/dom.js";
import { address, announce, slot, state } from "../../base/state.js";
import { money } from "../../base/note.js";
import { pools } from "../../base/read.js";
import {
  contextWindow as context_size,
  entry,
  fallbackModel,
} from "../../base/model.js";
import { move, switchModel } from "../../base/write.js";
import { accept, close, complete, isOpen, move as movePick } from "./path.js";
import { host } from "../../base/wire.js";
import {
  showCwdFeedback,
  updateEffort,
  updateRing,
  wireWidgets,
} from "./ring.js";

export function where() {
  const meta = state.session && state.session.meta;
  return (meta && meta.cwd) || (state.draft && state.draft.cwd) || "";
}

function models(picked) {
  const box = clear(at("model"));
  const list = (state.pools && state.pools.models) || [];
  if (!list.length) {
    box.append(el("option", { value: "", text: "no models" }));
    box.disabled = true;
    return;
  }
  box.disabled = false;
  for (const m of list) {
    box.append(
      el("option", {
        value: m.address,
        text: m.provider + " · " + (m.name || m.address),
      }),
    );
  }
  if (picked && !list.some((m) => m.address === picked)) {
    box.append(el("option", { value: picked, text: picked }));
  }
  box.value = picked || list[0].address;
}

export function draw() {
  const held = state.session;
  at("main").dataset.empty = held ? "0" : "1";
  const field = at("cwd");
  if (field.readOnly) {
    const path = where();
    if (field.value !== path) field.value = path;
    field.scrollLeft = field.scrollWidth;
    field.title = path || "double click to change the directory";
  }
  models(address());
  updateEffort(held);
  const usage = held && held.usage;
  at("spend").textContent = usage && usage.cny ? money(usage.cny) : "";
  updateRing(usage && usage.context, context_size());
  const bar = at("topbar").querySelector(".grow");
  bar.querySelector("h1").textContent = held
    ? (held.meta && held.meta.name) || "session " + held.id
    : "mypi";
  const acts = clear(at("session-acts"));
  if (held && held.busy) {
    acts.append(el("span", { class: "meta", text: "running" }));
  }
  at("stop").hidden = !(held && held.busy);
  // The queue is written as its own lines, one per held message: one line of joined text grows into a paragraph
  // inside a box that must stay the box's own height.
  const queue = at("queue");
  const said = (held && held.queued) || [];
  if (queue.dataset.count !== String(said.length)) {
    queue.dataset.count = String(said.length);
    queue.replaceChildren(
      ...said.map((text, at) =>
        el("div", { class: "line", text: (at + 1) + " · " + text })
      ),
    );
  }
}

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
  wireWidgets();
  at("model").addEventListener("change", (e) => {
    const picked = e.target.value;
    if (!picked) return;
    if (state.session) switchModel(picked);
    else {
      const held = slot();
      held.model = picked;
      held.effort = "auto";
      announce();
    }
    entry(picked);
  });

  at("cwd").addEventListener("dblclick", edit);
  at("cwd").addEventListener("blur", settle);
  at("cwd").addEventListener("keydown", handleCwdKey);
  at("cwd").addEventListener("input", () => complete(at("cwd")));
  // The field keeps the tail of its path in view itself, so a window grab shows which directory it is in.
  at("cwd").addEventListener("scroll", () => {
    const field = at("cwd");
    if (field.readOnly) field.scrollLeft = field.scrollWidth;
  }, { passive: true });
  // Hold the field's view on the tail of its path: the last folders say where a session stands.
  new ResizeObserver(() => {
    const field = at("cwd");
    if (!field.readOnly) return;
    field.title = field.value || "";
    field.scrollLeft = field.scrollWidth;
  }).observe(at("cwd"));
}

export async function boot() {
  slot();
  if (!state.draft.cwd) {
    try {
      const listed = await host("list", {});
      if (listed.dir) state.draft.cwd = listed.dir;
    } catch {}
  }
  await pools();
  const picked = address() || (await fallbackModel());
  await entry(picked);
  if (!state.draft.model) state.draft.model = picked;
  announce();
}
