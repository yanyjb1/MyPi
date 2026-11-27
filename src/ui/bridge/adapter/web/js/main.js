// The page's boot: register the panels, wire the controls, ask the first rows, and redraw once. Everything below is
// one region's own entry point; nothing here knows what a region holds.

import { at } from "./core/template.js";
import { closePanel, kept, watch as watchState } from "./core/store.js";
import { interrupt, refresh, remove } from "./core/api.js";
import * as theme from "./core/theme.js";
import * as rail from "./components/rail.js";
import * as capsule from "./components/composer/mods.js";
import * as input from "./components/composer/input.js";
import { keys, submit } from "./components/composer/keys.js";
import { send } from "./components/composer/talk.js";
import { enter, fresh, head } from "./components/chat/list.js";
import * as sheet from "./components/chat/sheet.js";
import { follow, latest } from "./components/chat/scroll.js";
import * as dial from "./components/dial.js";
import { draw as drawPanels, PanelRegistry, toggle as togglePanel } from "./components/panels.js";
import { register as registerUsage } from "../panels/usage/usage.js";
import { register as registerMemo } from "../panels/memo/memo.js";
import * as citation from "../panels/citation/citation.js";

registerUsage(PanelRegistry);
registerMemo(PanelRegistry);
citation.register(PanelRegistry);

// A burst of stream pieces moves the tail once: the redraw waits for the frame, not for every piece.
let pending = null;

// A piece moves the tail, a bill moves the ring and the drawer, and the rest of a change moves the page.
function paint(reason) {
  if (reason === "bill") {
    capsule.draw();
    drawPanels();
    return;
  }
  if (reason === "tail") {
    sheet.tail();
    return;
  }
  rail.draw();
  head();
  capsule.draw();
  sheet.draw();
  drawPanels();
  dial.draw();
}

// A stream piece moves the tail alone; everything else is a whole page's worth of change.
function draw(reason) {
  if (reason !== "tail") {
    if (pending) {
      cancelAnimationFrame(pending);
      pending = null;
    }
    paint(reason);
    return;
  }
  if (pending) return;
  pending = requestAnimationFrame(() => {
    pending = null;
    paint("tail");
  });
}

watchState(draw);
citation.wire();
keys();
rail.wire();
input.wire();
capsule.wire();
follow();
dial.wire();
theme.boot();
theme.wire();

// Three columns need a reading column between them. Narrower than that, the middle keeps the width and the two
// drawers come in over it, one at a time, asked for by their own chevrons: a drawer the page left open shuts on the
// way in, so the narrow window never opens on a panel covering what the reader was reading.
const roomy = matchMedia("(min-width: 71rem)");
function fold() {
  if (roomy.matches) return;
  document.body.dataset.rail = "closed";
  closePanel();
}
roomy.addEventListener("change", fold);
fold();

at("send").addEventListener("click", submit);
at("stop").addEventListener("click", () => interrupt());
at("rail-open").addEventListener("click", () => {
  document.body.dataset.rail = "open";
});
at("rail-close").addEventListener("click", () => {
  document.body.dataset.rail = "closed";
});
// The handle only opens and shuts the drawer; which panel stands in it is the command's own business.
at("panel-toggle").addEventListener("click", () => togglePanel());
at("panel-close").addEventListener("click", () => closePanel());
at("backdrop").addEventListener("click", () => {
  document.body.dataset.rail = "closed";
  closePanel();
});
at("jump-latest").addEventListener("click", () => latest());

draw();
input.boot().then(async () => {
  // The rail has nothing to draw until the rows are in hand, so the list is asked for before anything on it is entered.
  await refresh();
  // A tab remembers the conversation it stood in; one that remembers none opens a draft rather than the newest row.
  const id = kept();
  if (!id || !(await enter(id))) fresh();
  at("say").focus();
});

export { fresh, interrupt, remove, send, submit };
