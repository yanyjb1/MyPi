// The page's boot: register the panels, wire the controls, ask the first rows, and redraw once.

import { at } from "./base/dom.js";
import { closePanel, state, watch as watchState } from "./base/state.js";
import { bill, refresh } from "./base/read.js";
import { enter } from "./chat/history/enter.js";
import { interrupt, remove } from "./base/write.js";
import { fresh } from "./rail/draft.js";
import { send } from "./chat/composer/talk.js";
import { keys, submit } from "./chat/composer/key.js";
import * as rail from "./rail/rail.js";
import * as bar from "./chat/composer/bar.js";
import * as sheet from "./chat/history/sheet.js";
import * as place from "./chat/history/place.js";
import * as dial from "./chat/dial/dial.js";
import * as theme from "./base/theme.js";
import {
  draw as drawPanels,
  PanelRegistry,
  toggle as togglePanel,
} from "./panel/panels.js";
import { register as registerUsage } from "./panel/plugins/usage.js";

// The pieces a panel may ask for: the page's own writes, and the reads its rows come from.
const page = { bill };

registerUsage(PanelRegistry, page);

// A burst of stream pieces moves the tail once: the redraw waits for the frame, not for every piece.
let pending = null;

// A piece moves the tail, a bill moves the ring and the drawer, and the rest of a change moves the page.
function paint(reason) {
  if (reason === "bill") {
    bar.draw();
    drawPanels();
    return;
  }
  if (reason === "tail") {
    sheet.tail();
    return;
  }
  rail.draw();
  bar.draw();
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
keys();
rail.wire();
bar.wire();
place.follow();
dial.wire();
theme.boot();
theme.wire();

// Three columns need a reading column between them. Narrower than that, the middle keeps the width and the two
// drawers come in over it, one at a time, asked for by their own chevrons: a drawer the page left open shuts on
// the way in, so the narrow window never opens on a panel covering what the reader was reading.
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
at("jump-latest").addEventListener("click", () => place.latest());

draw();
bar.boot().then(async () => {
  await refresh();
  if (state.active) await enter(state.active);
  else fresh();
  at("say").focus();
});

export { fresh, interrupt, page, remove, send, submit };
