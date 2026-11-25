// The setup panel: the rows of a conversation's own header, the pool, and compaction.

import { el } from "../../base/dom.js";
import { state } from "../../base/state.js";
import { rungs } from "../../base/model.js";

export function register(PanelRegistry, page) {
  PanelRegistry.register({
    id: "setup",
    label: "setup",
    frame() {
      const box = el("div");
      const name = el("input", { placeholder: "name", spellcheck: "false" });
      const path = el("input", {
        placeholder: "directory",
        spellcheck: "false",
      });
      const facts = el("dl");
      const cells = new Map();
      for (const key of ["directory", "effort", "blocks"]) {
        const value = el("dd", { text: "—" });
        cells.set(key, value);
        facts.append(el("dt", { text: key }), value);
      }
      const models = el("select");
      box.append(
        facts,
        el("h2", { text: "name" }),
        el(
          "div",
          { class: "row" },
          name,
          el("button", {
            class: "btn",
            text: "set",
            onclick: () => page.rename(name.value),
          }),
        ),
        el("h2", { text: "directory" }),
        el(
          "div",
          { class: "row" },
          path,
          el("button", {
            class: "btn",
            text: "move",
            onclick: () => page.move(path.value),
          }),
        ),
        el("h2", { text: "model" }),
        el(
          "div",
          { class: "row" },
          models,
          el("button", {
            class: "btn",
            text: "this session",
            onclick: () => page.switchModel(models.value),
          }),
          el("button", {
            class: "btn",
            text: "default",
            onclick: () => page.defaultModel(models.value),
          }),
        ),
        el("h2", { text: "effort" }),
        el("div", { class: "row rungs" }),
        el("h2", { text: "compact" }),
        el(
          "div",
          { class: "row" },
          el("input", { placeholder: "focus (optional)", spellcheck: "false" }),
          el("button", {
            class: "btn danger",
            text: "now",
            onclick: () => page.compact(""),
          }),
        ),
      );
      box.cells = cells;
      box.models = models;
      box.rungs = box.querySelector(".rungs");
      return box;
    },
    load: () => page.pools(),
    fill(box, held) {
      const meta = (held && held.meta) || {};
      box.cells.get("directory").textContent = meta.cwd || "—";
      box.cells.get("effort").textContent = (held && held.effort) || "auto";
      box.cells.get("blocks").textContent = String(held ? held.blocks.size : 0);
      const pool = (state.pools && state.pools.models) || [];
      const picked = (meta.provider ? meta.provider + ":" : "") +
        (meta.model || "");
      box.models.replaceChildren();
      for (const model of pool) {
        box.models.append(
          el("option", {
            value: model.address,
            text: model.provider + " · " + (model.name || model.address),
          }),
        );
      }
      box.models.value = picked;
      // The rungs are the ones this model declares; a model without them gets no row at all.
      const list = rungs();
      box.rungs.replaceChildren();
      box.rungs.hidden = list.length < 2;
      for (const rung of list) {
        box.rungs.append(
          el("button", {
            class: "btn",
            "data-on": ((held && held.effort) || "auto") === rung ? "1" : "0",
            text: rung,
            onclick: () => page.setRung(rung),
          }),
        );
      }
    },
  });
}
