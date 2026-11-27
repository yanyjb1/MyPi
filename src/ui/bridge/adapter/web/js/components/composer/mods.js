// The capsule under the box: the directory, the branch, the ring of context, what the session has spent, and the
// engine the model is picked on. Each widget redraws itself alone; the box above them is `input.js`'s business.

import { at, build } from "../../core/template.js";
import { compact, contextWindow, rungs, setRung, switchModel } from "../../core/api.js";
import { address, announce, slot, state } from "../../core/store.js";
import { money } from "../../core/format.js";

const CIRCUMFERENCE = 56.5; // 2 * PI * 9 ≈ 56.55
let hideTimer = 0;

function updateRing(context, size) {
  const ring = at("context-ring");
  if (!ring) return;
  const arc = at("context-arc");
  if (!context || !size) {
    ring.title = "Context usage: 0% · Click to compact";
    if (arc) arc.style.strokeDashoffset = String(CIRCUMFERENCE);
    return;
  }
  const ratio = Math.min(Math.max(context / size, 0), 1);
  const pct = Math.round(ratio * 100);
  ring.title = `Context: ${pct}% (${context}/${size}) · Click to compact`;
  if (arc) arc.style.strokeDashoffset = String(CIRCUMFERENCE * (1 - ratio));
}

export function showCwdFeedback(success) {
  const badge = at("cwd-status");
  if (!badge) return;
  clearTimeout(hideTimer);
  badge.textContent = success ? "(changed)" : "(unchanged)";
  badge.className = "cwd-status " + (success ? "changed" : "unchanged");
  badge.dataset.show = "1";
  hideTimer = setTimeout(() => {
    badge.dataset.show = "0";
  }, 1600);
}

function updateEffort(held) {
  const btn = at("effort-btn");
  const divider = at("capsule-divider");
  if (!btn) return;
  const list = rungs();
  const capable = list.length >= 2;
  btn.hidden = !capable;
  if (divider) divider.hidden = !capable;
  // The capsule itself reads the same fact, so a model with no effort is a capsule with no sleeve for one: the
  // hidden button alone left the capsule as wide as the widest model name, with nothing inside that width.
  const capsule = at("engine-capsule");
  if (capsule) capsule.dataset.effort = capable ? "1" : "0";
  if (!capable) return;
  const live = (held && held.effort) || (state.draft && state.draft.effort) ||
    "auto";
  btn.dataset.effort = live;
  const said = at("effort-text");
  if (said) said.textContent = live;
  btn.title = `Thinking effort: ${live} · Scroll or Click to cycle`;
}

function cycleEffort(delta = 1) {
  const list = rungs();
  if (list.length < 2) return;
  const held = state.session;
  const live = (held && held.effort) || (state.draft && state.draft.effort) ||
    "auto";
  const idx = Math.max(list.indexOf(live), 0);
  const next = list[(idx + delta + list.length) % list.length];
  const btn = at("effort-btn");
  if (btn) btn.dataset.effort = next;
  const said = at("effort-text");
  if (said) said.textContent = next;
  setRung(next);
}

// The model picker: one option a model, the one in force selected.
function models(picked) {
  const box = at("model");
  box.replaceChildren();
  const list = (state.pools && state.pools.models) || [];
  if (!list.length) {
    const one = build("tpl-option");
    one.textContent = "no models";
    box.append(one);
    box.disabled = true;
    return;
  }
  box.disabled = false;
  for (const m of list) {
    const one = build("tpl-option");
    one.value = m.address;
    one.textContent = m.provider + " · " + (m.name || m.address);
    box.append(one);
  }
  if (picked && !list.some((m) => m.address === picked)) {
    const one = build("tpl-option");
    one.value = picked;
    one.textContent = picked;
    box.append(one);
  }
  box.value = picked || list[0].address;
}

export function draw() {
  const held = state.session;
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
  updateRing(usage && usage.context, contextWindow());
  // The queue is written as its own lines, one per held message: one line of joined text grows into a paragraph
  // inside a box that must stay the box's own height.
  const queue = at("queue");
  const said = (held && held.queued) || [];
  if (queue.dataset.count !== String(said.length)) {
    queue.dataset.count = String(said.length);
    queue.replaceChildren(
      ...said.map((line, at) => {
        const node = build("tpl-queue-line");
        node.textContent = at + 1 + " · " + line;
        return node;
      }),
    );
  }
}

// Where the session stands, as a path: the row's own, the draft's while it has no row, and nothing before that.
export function where() {
  const meta = state.session && state.session.meta;
  return (meta && meta.cwd) || (state.draft && state.draft.cwd) || "";
}

export function wire() {
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
  });

  const ring = at("context-ring");
  if (ring) ring.addEventListener("click", () => compact());

  const btn = at("effort-btn");
  if (btn) {
    btn.addEventListener("click", (e) => {
      e.stopPropagation();
      cycleEffort(1);
    });
    btn.addEventListener("wheel", (e) => {
      e.preventDefault();
      e.stopPropagation();
      cycleEffort(e.deltaY > 0 ? 1 : -1);
    }, { passive: false });
  }

  document.addEventListener("keydown", (e) => {
    // Avoid browser Alt+E menu collision on Linux/Firefox by supporting Ctrl+Alt+E.
    if (e.ctrlKey && e.altKey && (e.key === "e" || e.key === "E")) {
      e.preventDefault();
      cycleEffort(1);
    }
  });
}

