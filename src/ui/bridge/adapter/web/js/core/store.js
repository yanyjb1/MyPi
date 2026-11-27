// What the page knows: every conversation visited, the one on screen, the draft, and the panel. A change says how
// much of the page it moves, and the page answers its watchers with that one word.

import { at } from "./template.js";

export const state = {
  rows: [],
  active: null,
  session: null,
  draft: null,
  panel: { open: false, tab: null },
  pools: null,
  refusal: null,
  streaming: false,
  follow: null,
  pinned: true,
  byId: new Map(),
};

const watchers = new Set();

export function watch(fn) {
  watchers.add(fn);
  fn();
}

// One stream piece touches the live tail alone; a bill moves the ring and the drawer; anything else the whole page.
export function announce(reason = "all") {
  for (const fn of watchers) fn(reason);
}

// One conversation's own memory: the round list is whole, the bodies and their nodes are a page.
export function blank(id) {
  return {
    id,
    meta: null,
    structure: [],
    blocks: new Map(),
    view: new Map(),
    heads: null,
    refusalNode: null,
    span: null,
    anchor: null,
    asking: new Set(),
    live: [],
    liveNode: null,
    busy: false,
    liveTurn: null,
    effort: "auto",
    usage: null,
    queued: [],
  };
}

// What a session would open with, kept while its row does not exist yet.
export function slot() {
  if (!state.draft) state.draft = { cwd: "", model: null, effort: "auto" };
  return state.draft;
}

export function ensure(id) {
  let held = state.byId.get(id);
  if (!held) {
    held = blank(id);
    state.byId.set(id, held);
  }
  return held;
}

// One refusal, said in the transcript; the next entry into a conversation clears it.
export function failed(e) {
  state.refusal = e instanceof Error ? e.message : String(e);
  announce();
}

export function dismiss() {
  state.refusal = null;
  announce();
}

export function openPanel(tab) {
  state.panel.open = true;
  if (tab) state.panel.tab = tab;
  announce();
}

export function closePanel() {
  state.panel.open = false;
  announce();
}

// Another conversation: the drawer's owner spoke about rows that are gone, so it is put away whole.
export function dropPanel() {
  state.panel.open = false;
  state.panel.tab = null;
  announce();
}

// The model this page is about to speak for, as `<provider>:<model>`.
export function address() {
  const meta = state.session && state.session.meta;
  if (meta && meta.model) {
    return (meta.provider ? meta.provider + ":" : "") + meta.model;
  }
  return (state.draft && state.draft.model) || null;
}

// The directory a word of the composer is completed against.
export function cwd() {
  const meta = state.session && state.session.meta;
  if (meta && meta.cwd) return meta.cwd;
  if (state.draft && state.draft.cwd) return state.draft.cwd;
  const box = at("cwd");
  return box ? box.value.trim() : "";
}

// The conversation this tab last stood in, kept in the tab's own storage: a reload comes back to it, while a second
// tab opens on a draft of its own, because it brings no storage with it.
const KEPT = "mypi.session";

export function keep(id) {
  try {
    if (id) sessionStorage.setItem(KEPT, String(id));
    else sessionStorage.removeItem(KEPT);
  } catch {
    // A browser that keeps no storage reloads into a fresh draft: forgetful, never wrong.
  }
}

export function kept() {
  try {
    const id = Number(sessionStorage.getItem(KEPT));
    return Number.isSafeInteger(id) && id > 0 ? id : null;
  } catch {
    return null;
  }
}
