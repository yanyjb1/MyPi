// The context ring, cwd feedback, and effort widgets in the composer deck.

import { at } from "../../base/dom.js";
import { compact, setRung } from "../../base/write.js";
import { rungs } from "../../base/model.js";
import { state } from "../../base/state.js";

const CIRCUMFERENCE = 56.5; // 2 * PI * 9 ≈ 56.55
let hideTimer = 0;

export function updateRing(context, size) {
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

export function updateEffort(held) {
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
  const text = at("effort-text");
  if (text) text.textContent = live;
  btn.title = `Thinking effort: ${live} · Scroll or Click to cycle`;
}

export function cycleEffort(delta = 1) {
  const list = rungs();
  if (list.length < 2) return;
  const held = state.session;
  const live = (held && held.effort) || (state.draft && state.draft.effort) ||
    "auto";
  const idx = Math.max(list.indexOf(live), 0);
  const next = list[(idx + delta + list.length) % list.length];
  const btn = at("effort-btn");
  if (btn) btn.dataset.effort = next;
  const text = at("effort-text");
  if (text) text.textContent = next;
  setRung(next);
}

export function wireWidgets() {
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
    // Avoid browser Alt+E menu collision on Linux/Firefox by supporting Ctrl+Alt+E
    if (e.ctrlKey && e.altKey && (e.key === "e" || e.key === "E")) {
      e.preventDefault();
      cycleEffort(1);
    }
  });
}
