// The tree panel: every round with the user's words in it, a sight line to any of them, and rewind.

import { el } from "../../base/dom.js";
import { short } from "../../base/note.js";
import { said } from "../../chat/history/rounds.js";

export function register(PanelRegistry, page) {
  PanelRegistry.register({
    id: "tree",
    label: "tree",
    frame() {
      const box = el("div");
      const note = el("div", { class: "hint" });
      const list = el("div");
      box.append(note, list);
      box.note = note;
      box.list = list;
      return box;
    },
    fill(box, held) {
      const rounds = (held && held.structure) || [];
      box.note.textContent = rounds.length
        ? rounds.length + " rounds · click one to go there"
        : "no rounds";
      box.list.replaceChildren();
      for (const round of rounds) {
        const words = said(held, round);
        const ids = (round.ranges || []).map(([lo, hi]) => lo + "–" + hi).join(
          " ",
        );
        box.list.append(
          el(
            "div",
            { class: "pick", onclick: () => page.jump(round.turn) },
            el("span", { class: "dim", text: "#" + round.turn }),
            el("span", {
              class: "fill",
              text: (round.compact ? "compaction · " : "") +
                (words ? short(words.replace(/\s+/g, " "), 70) : ids),
            }),
            el("button", {
              class: "btn danger",
              text: "rewind",
              onclick: (event) => {
                event.stopPropagation();
                const last = (round.ranges || []).slice(-1)[0];
                if (last) page.askRewind(round.turn, last[0]);
              },
            }),
          ),
        );
      }
    },
  });
}
