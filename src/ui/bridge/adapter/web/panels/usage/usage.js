// The usage panel: the TUI's own bill laid out for a drawer. It brings its own markup and its own sheet, and keeps
// one fact of its own — whether the long tail of models is open.

import { fetchTemplate } from "../../js/core/template.js";
import { bill as readBill } from "../../js/core/api.js";
import { cards, foot, head, row, split } from "./bill.js";

const HERE = "/panels/usage/";

export function register(PanelRegistry) {
  PanelRegistry.register({
    id: "usage",
    label: "usage",
    css: HERE + "usage.css",
    template: () => fetchTemplate(HERE + "usage.html"),
    load: () => readBill(),
    fill(box, held) {
      const bill = held && held.usage;
      const slots = {
        head: box.querySelector('[data-ref="head"]'),
        split: box.querySelector('[data-ref="split"]'),
        cards: box.querySelector('[data-ref="cards"]'),
        foot: box.querySelector('[data-ref="foot"]'),
      };
      if (!bill || !(bill.groups || []).length) {
        for (const slot of Object.values(slots)) slot.replaceChildren();
        const empty = document.createElement("div");
        empty.className = "empty";
        empty.textContent = "no usage yet";
        slots.head.append(empty);
        return;
      }
      const rows = bill.groups.map(row);
      const pool = (bill.subagent_groups || []).map(row);
      slots.head.replaceChildren(head(bill, rows));
      slots.split.replaceChildren(split(rows));
      slots.foot.replaceChildren(foot(bill, rows));
      // One click on the tail redraws the cards alone: the whole drawer is not a click's worth of work.
      const ask = () => {
        const opened = box.dataset.opened !== "1";
        box.dataset.opened = opened ? "1" : "0";
        slots.cards.replaceChildren(cards(rows, pool, opened, ask));
      };
      slots.cards.replaceChildren(
        cards(rows, pool, box.dataset.opened === "1", ask),
      );
    },
  });
}