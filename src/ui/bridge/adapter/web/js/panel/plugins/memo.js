// The memo panel: one form that writes a note, the notes themselves, and the one memo whose versions are open.

import { el } from "../../base/dom.js";
import { short } from "../../base/note.js";

export function register(PanelRegistry, page) {
  PanelRegistry.register({
    id: "memo",
    label: "memo",
    frame() {
      const box = el("div");
      const form = compose(page);
      const list = el("div");
      const read = el("div");
      box.append(form, list, read);
      box.form = form;
      box.list = list;
      box.read = read;
      return box;
    },
    load: () => page.memos(),
    fill(box, held) {
      listing(box, held, page);
      const open = held && held.read;
      box.read.hidden = !open;
      if (open) versions(box.read, open, page);
    },
  });
}

// The one form: a name, the scope it lands in, its text, and the write that sends it. A name already in the
// list appends a version to that memo; a new one starts it. The button says which.
function compose(page) {
  const name = el("input", {
    type: "text",
    placeholder: "name",
    spellcheck: "false",
    "aria-label": "memo name",
  });
  const scope = el(
    "select",
    { "aria-label": "memo scope" },
    el("option", { value: "global", text: "global" }),
    el("option", { value: "current", text: "current" }),
  );
  const text = el("textarea", {
    placeholder: "what to remember",
    "aria-label": "memo content",
  });
  const save = el("button", { class: "btn primary", text: "write" });
  save.addEventListener("click", () => {
    const said = name.value.trim();
    if (!said) return;
    page.writeMemo("add", scope.value, said, text.value).then(() => {
      name.value = "";
      text.value = "";
    });
  });
  return el("div", { class: "stack" }, name, scope, text, save);
}

// Every memo this page may read, newest content first: one card each, and a click opens its versions.
function listing(box, held, page) {
  const rows = (held && held.memos) || [];
  box.list.replaceChildren();
  if (!rows.length) {
    box.list.append(el("div", { class: "empty", text: "no memos" }));
    return;
  }
  for (const memo of rows) {
    box.list.append(
      el(
        "div",
        { class: "card", onclick: () => page.readMemo(memo.scope, memo.name) },
        el("span", { text: memo.name + " · " + memo.scope }),
        el("span", { class: "dim", text: short(memo.content, 160) }),
      ),
    );
  }
}

// One memo's versions, newest last, and the one write that deletes it.
function versions(box, read, page) {
  box.replaceChildren(
    el(
      "div",
      { class: "row" },
      el("span", { class: "hint", text: read.scope + " · " + read.name }),
      el("button", {
        class: "btn",
        text: "back",
        onclick: () => page.closeMemo(),
      }),
    ),
  );
  for (const version of read.versions || []) {
    box.append(
      el(
        "div",
        { class: "pick" },
        el("span", { class: "dim", text: "#" + version.id }),
        el("span", {
          class: "fill",
          text: short((version.content || "").replace(/\s+/g, " "), 60),
        }),
      ),
    );
  }
  const last = (read.versions || []).slice(-1)[0];
  box.append(
    el(
      "div",
      { class: "card" },
      el("div", { text: (last && last.content) || "" }),
    ),
    el(
      "div",
      { class: "row" },
      el("button", {
        class: "btn danger",
        text: "del",
        onclick: () => page.writeMemo("del", read.scope, read.name, ""),
      }),
    ),
  );
}
