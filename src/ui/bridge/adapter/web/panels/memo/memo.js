// The memo panel: this conversation's notes beside the global ones, each with every version it has had. The host
// only lays the drawer down; what stands inside it is this file's own state until its next read.

import { memosRead, memosWrite } from "../../js/core/api.js";
import { failed, state } from "../../js/core/store.js";
import { build, fetchTemplate, ref, text } from "../../js/core/template.js";
import { short } from "../../js/core/format.js";
import { body as render } from "../../js/render/markdown.js";

const HERE = "/panels/memo/";

// How tall a version may stand before it is folded: past this the card says there is more.
const FOLD = 180;
// The width at which the drawer is a column rather than a sheet, so the versions lie one under another.
const ROOMY = "(min-width: 71rem)";

// What the drawer is showing: the notes, or one name's versions.
let view = "list";
// The note whose versions stand.
let open = null;
// Every name either scope holds, as the door gave them: one row each, carrying its newest text.
let rows = [];
// The versions of `open`, oldest first.
let versions = [];
// The scope whose new-note editor stands, or nothing.
let creating = null;
// Whether the editor for the next version stands.
let appending = false;
// The row asked to be deleted a second time, as `<scope>/<name>`.
let asking = null;
// The node the host laid down.
let box = null;
// What is drawn, so a redraw of the same thing leaves a reader's typing alone.
let stamp = "";

// A current note hangs on a block of this conversation, so a conversation without one holds none.
function alive(held) {
  return Boolean(held && held.structure && held.structure.length);
}

// The conversation a read or write names, when there is one.
function scope_of() {
  const held = state.session;
  if (!alive(held)) return {};
  return { session: held.id };
}

// Every name in both scopes, and the versions of the one on screen when it is.
async function reload() {
  try {
    const answer = await memosRead({ op: "list", ...scope_of() });
    rows = answer.memos || [];
  } catch (e) {
    rows = [];
    failed(e);
  }
  if (view === "detail" && open) await readVersions();
  paint();
}

async function readVersions() {
  try {
    const answer = await memosRead({
      op: "history",
      scope: open.scope,
      name: open.name,
      ...scope_of(),
    });
    versions = answer.versions || [];
  } catch (e) {
    versions = [];
    failed(e);
  }
}

// One write, then the drawer reads itself again: the door answers a write with nothing worth keeping.
async function write(ask) {
  try {
    await memosWrite({ ...ask, ...scope_of() });
    return true;
  } catch (e) {
    failed(e);
    return false;
  }
}

// What is drawn now, as one string: the same words mean the reader's own editor is left standing.
function mark() {
  return [
    alive(state.session) ? "1" : "0",
    view,
    open ? open.scope + "/" + open.name : "",
    creating || "",
    appending ? "1" : "0",
    asking || "",
    JSON.stringify(rows),
    versions.map((one) => one.id + ":" + String(one.content).length).join(","),
  ].join("|");
}

function paint(force) {
  if (!box) return;
  const said = mark();
  if (!force && said === stamp) return;
  stamp = said;
  box.dataset.view = view;
  box.replaceChildren(view === "detail" ? detail() : list());
  if (view === "detail") {
    requestAnimationFrame(() => {
      measure();
      snap();
    });
  }
}

function list() {
  const root = document.createElement("div");
  root.className = "memo-list";
  const held = alive(state.session);
  for (const scope of ["current", "global"]) {
    if (scope === "current" && !held) continue;
    root.append(group(scope));
  }
  return root;
}

function group(scope) {
  const node = build("tpl-memo-group");
  text(node, "title", scope === "current" ? "Current · this session" : "Global · everywhere");
  const mine = rows.filter((one) => one.scope === scope);
  text(node, "count", String(mine.length));
  const body = ref(node, "body");
  for (const row of mine) body.append(item(row));
  if (creating === scope) body.append(editor(scope, null));
  else {
    body.append(
      dashed("+ new note", () => {
        creating = scope;
        paint(true);
      }),
    );
  }
  return node;
}

function item(row) {
  const node = build("tpl-memo-item");
  const key = row.scope + "/" + row.name;
  text(node, "name", row.name);
  text(node, "preview", preview(row.content));
  const copy = node.querySelector('[data-act="copy"]');
  copy.addEventListener("click", () => flash(copy, row.content));
  node.querySelector('[data-act="open"]').addEventListener("click", () => {
    view = "detail";
    open = { scope: row.scope, name: row.name };
    versions = [];
    appending = false;
    asking = null;
    reload();
  });
  const wipe = node.querySelector('[data-act="del"]');
  wipe.dataset.asking = asking === key ? "1" : "0";
  wipe.addEventListener("click", async () => {
    if (asking !== key) {
      asking = key;
      paint(true);
      return;
    }
    asking = null;
    const gone = await write({ op: "del", scope: row.scope, name: row.name });
    if (gone) await reload();
  });
  return node;
}

function dashed(said, run) {
  const node = build("tpl-memo-dashed");
  node.textContent = said;
  node.addEventListener("click", run);
  return node;
}

function editor(scope, name) {
  const node = build("tpl-memo-editor");
  const field = ref(node, "name");
  if (name) {
    field.value = name;
    field.hidden = true;
  }
  const said = ref(node, "said");
  const save = ref(node, "save");
  if (name) save.textContent = "write v" + (versions.length + 1);
  node.querySelector('[data-act="cancel"]').addEventListener("click", () => shut(name));
  save.addEventListener("click", async () => {
    const key = name || field.value.trim();
    if (!name && !/^\S+$/.test(key)) {
      said.textContent = "a name is one word";
      return;
    }
    const whole = ref(node, "content").value.trim();
    if (!whole) {
      said.textContent = "nothing to write";
      return;
    }
    const ok = await write({ op: name ? "mod" : "add", scope, name: key, content: whole });
    if (!ok) return;
    if (name) appending = false;
    else creating = null;
    await reload();
  });
  return node;
}

// An editor put away: whichever one it was stops standing, and the drawer is drawn again.
function shut(name) {
  if (name) appending = false;
  else creating = null;
  paint(true);
}

function detail() {
  const node = build("tpl-memo-detail");
  text(node, "scope", open.scope);
  text(node, "title", open.name);
  text(node, "pager", pagerOf(1));
  node.querySelector('[data-act="back"]').addEventListener("click", () => {
    view = "list";
    open = null;
    versions = [];
    appending = false;
    paint(true);
  });
  const track = ref(node, "track");
  versions.forEach((one, at) => track.append(card(one, at)));
  const slot = document.createElement("div");
  slot.className = "memo-append";
  if (appending) slot.append(editor(open.scope, open.name));
  else {
    slot.append(
      dashed("+ write v" + (versions.length + 1), () => {
        appending = true;
        paint(true);
      }),
    );
  }
  track.append(slot);
  track.addEventListener(
    "scroll",
    () => {
      text(node, "pager", pagerOf(at(track)));
    },
    { passive: true },
  );
  return node;
}

// Which card the sheet is showing, as the head says it.
function at(track) {
  if (roomy()) return 1;
  const wide = track.clientWidth || 1;
  return Math.round(track.scrollLeft / wide) + 1;
}

function pagerOf(at) {
  const total = versions.length;
  if (at > total) return "new · " + total + "v";
  return at + " / " + total;
}

function card(one, at) {
  const node = build("tpl-memo-version");
  const newest = at === versions.length - 1;
  node.dataset.latest = newest ? "1" : "0";
  text(node, "badge", "v" + (at + 1) + (newest ? " · latest" : ""));
  text(node, "ts", short(String(one.ts || ""), 16));
  const copy = node.querySelector('[data-act="copy"]');
  copy.addEventListener("click", () => flash(copy, one.content));
  ref(node, "body").innerHTML = render(one.content);
  ref(node, "expand").addEventListener("click", () => {
    const wrap = ref(node, "wrap");
    const folded = wrap.dataset.folded !== "1";
    wrap.dataset.folded = folded ? "1" : "0";
    ref(node, "expand").textContent = folded ? "show all" : "less";
  });
  return node;
}

// A version taller than the card folds, and the fold's own button is the only thing that unfolds it.
function measure() {
  if (!box) return;
  for (const node of box.querySelectorAll(".memo-ver")) {
    const wrap = ref(node, "wrap");
    const more = ref(node, "expand");
    const long = wrap.scrollHeight > FOLD;
    node.dataset.long = long ? "1" : "0";
    if (!long) {
      wrap.dataset.folded = "0";
      more.hidden = true;
      continue;
    }
    if (wrap.dataset.folded !== "0") wrap.dataset.folded = "1";
    more.hidden = false;
    more.textContent = wrap.dataset.folded === "1" ? "show all" : "less";
  }
}

// The sheet opens on the newest version: the older ones are behind a finger's left swipe.
function snap() {
  if (roomy() || !box) return;
  const track = box.querySelector(".memo-versions");
  if (!track) return;
  const newest = track.children[versions.length - 1];
  if (!newest) return;
  track.scrollLeft = newest.getBoundingClientRect().left - track.getBoundingClientRect().left + track.scrollLeft;
}

function roomy() {
  return matchMedia(ROOMY).matches;
}

// A copied text says so on the button that copied it, then says nothing again.
function flash(node, content) {
  navigator.clipboard?.writeText(String(content ?? "")).catch(() => {});
  node.dataset.copied = "1";
  setTimeout(() => {
    node.dataset.copied = "0";
  }, 1100);
}

// A row's newest text as one line: the marks that only mean something to a reader of the whole note go.
function preview(content) {
  return String(content ?? "")
    .replace(/[#`*>_~-]+/g, " ")
    .replace(/\s+/g, " ")
    .trim();
}

export function register(PanelRegistry) {
  PanelRegistry.register({
    id: "memo",
    label: "memo",
    css: HERE + "memo.css",
    template: () => fetchTemplate(HERE + "memo.html"),
    load: async () => {
      view = "list";
      open = null;
      versions = [];
      creating = null;
      appending = false;
      asking = null;
      stamp = "";
      await reload();
    },
    fill(node) {
      box = node;
      // A reader in the middle of writing a note is not interrupted by whatever moved elsewhere on the page.
      if (creating || appending) return;
      paint();
    },
  });
}
