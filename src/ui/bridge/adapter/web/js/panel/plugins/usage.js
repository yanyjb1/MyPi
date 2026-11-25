// The usage panel: the TUI's own bill, laid out for a drawer. Four figures at its head, where the tokens went, one
// card a model, and the session's foot. The columns are those of usage.rs; `bill.js` draws each part.

import { el } from "../../base/dom.js";
import { cards, foot, head, split } from "../bill.js";

// One model's share of the bill, decoded from a pool group the way usage.rs decodes it.
function row(group) {
  const prompt = Number(group.prompt_tokens || 0);
  const completion = Number(group.completion_tokens || 0);
  const cached = Number(group.cached_tokens || 0);
  const written = Number(group.cache_write_tokens || 0);
  return {
    address: group.address || group.model || "",
    rounds: Number(group.rounds || 0),
    prompt,
    completion,
    cached,
    written,
    total: prompt + completion,
    // Input tokens no cache reported: the five numbers are one partition of the total.
    input: Math.max(0, prompt - cached - written),
    price: group.price || null,
  };
}

export function register(PanelRegistry, page) {
  PanelRegistry.register({
    id: "usage",
    label: "usage",
    frame() {
      const box = el("div", { class: "bill" });
      // The long tail stays shut until it is asked for, and stays shut across a redraw. The four slots are held
      // under a name of their own: `part` and `head` are the element's own properties, not a place to keep ours.
      box.opened = false;
      box.slots = {
        head: el("div"),
        split: el("div"),
        cards: el("div"),
        foot: el("div"),
      };
      box.append(...Object.values(box.slots));
      return box;
    },
    load: () => page.bill(),
    fill(box, held) {
      const bill = held && held.usage;
      const slots = box.slots;
      if (!bill || !(bill.groups || []).length) {
        for (const slot of Object.values(slots)) slot.replaceChildren();
        slots.head.append(el("div", { class: "empty", text: "no usage yet" }));
        return;
      }
      const rows = bill.groups.map(row);
      const pool = (bill.subagent_groups || []).map(row);
      slots.head.replaceChildren(head(bill, rows));
      slots.split.replaceChildren(split(rows));
      slots.foot.replaceChildren(foot(bill, rows));
      // One click on the tail redraws the cards alone: the whole drawer is not a click's worth of work.
      const ask = () => {
        box.opened = !box.opened;
        slots.cards.replaceChildren(cards(rows, pool, box.opened, ask));
      };
      slots.cards.replaceChildren(cards(rows, pool, box.opened, ask));
    },
  });
}
