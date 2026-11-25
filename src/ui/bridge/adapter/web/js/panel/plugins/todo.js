// The todo panel: one list and one line of count, written over as the checklist changes.

import { el } from "../../base/dom.js";

export function register(PanelRegistry, page) {
  PanelRegistry.register({
    id: "todo",
    label: "todo",
    frame() {
      const box = el("div");
      const count = el("div", { class: "hint" });
      const list = el("ul");
      box.append(count, list);
      box.count = count;
      box.list = list;
      return box;
    },
    load: () => page.todos(),
    fill(box, held) {
      const rows = (held && held.todo) || [];
      const done = rows.filter((item) => item && item.done).length;
      box.count.textContent = rows.length
        ? done + " of " + rows.length + " done"
        : "no checklist";
      box.list.replaceChildren();
      for (const item of rows) {
        box.list.append(
          el("li", {
            class: "dim",
            text: (item && item.done ? "✓ " : "· ") +
              ((item && item.text) || ""),
          }),
        );
      }
    },
  });
}
