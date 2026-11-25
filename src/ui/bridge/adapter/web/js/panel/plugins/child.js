// The child panel: the subagents this conversation started, each one a conversation of its own.

import { el } from "../../base/dom.js";
import { short } from "../../base/note.js";

export function register(PanelRegistry, page) {
  PanelRegistry.register({
    id: "child",
    label: "child",
    frame() {
      const box = el("div");
      const refresh = el(
        "div",
        { class: "row" },
        el("button", {
          class: "btn",
          text: "refresh",
          onclick: () => page.kids(),
        }),
      );
      const list = el("div");
      box.append(refresh, list);
      box.list = list;
      return box;
    },
    load: () => page.kids(),
    fill(box, held) {
      const rows = (held && held.children) || [];
      box.list.replaceChildren();
      if (!rows.length) {
        box.list.append(el("div", { class: "empty", text: "no subagents" }));
        return;
      }
      for (const row of rows) {
        box.list.append(
          el(
            "div",
            { class: "pick", onclick: () => page.enter(row.id) },
            el("span", { class: "dim", text: "#" + row.id }),
            el("span", {
              class: "fill",
              text: short(row.name || row.first_user || "", 40),
            }),
            el("span", {
              class: "dim",
              text: row.running ? "running" : row.rounds + "r",
            }),
          ),
        );
      }
    },
  });
}
