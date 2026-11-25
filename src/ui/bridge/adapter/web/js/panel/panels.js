// The right drawer: one owner at a time, its own template, and only its own writes on top of it.

import { at, clear, el } from "../base/dom.js";
import { closePanel, openPanel, state } from "../base/state.js";

const plugins = new Map();
// Whose template stands in the drawer's body; empty when nobody's does.
let standing = null;
// The node that owner laid down, which every fill of it writes into.
let mounted = null;

export const PanelRegistry = {
  register(plugin) {
    if (!plugin || !plugin.id) return;
    const maker = plugin.template || plugin.frame;
    const updater = plugin.update || plugin.fill;
    if (typeof maker !== "function" || typeof updater !== "function") return;
    plugins.set(plugin.id, {
      ...plugin,
      frame: maker,
      fill: updater,
    });
  },
  unregister(id) {
    plugins.delete(id);
    if (standing === id) vacate();
  },
  plugins() {
    return [...plugins.values()];
  },
};

// A new owner takes the drawer, and what the last owner left goes with it.
export function give(id) {
  if (!plugins.has(id)) return;
  openPanel(id);
}

// One click: the drawer opens on the owner it last held, or shuts. It never picks an owner itself.
export function toggle() {
  if (state.panel.open) closePanel();
  else if (state.panel.tab) openPanel(state.panel.tab);
}

// The standing owner speaks again: it alone decides whether that is the whole template or one number in it.
export function refresh(id) {
  if (
    !state.panel.open || state.panel.tab !== id || standing !== id || !mounted
  ) return;
  plugins.get(id).fill(mounted, state.session);
}

// Nobody's owner stands: the drawer keeps an empty body and forgets the one it had.
export function vacate() {
  if (standing) {
    clear(at("panel-body"));
    standing = null;
    mounted = null;
  }
}

export function draw() {
  document.body.dataset.panel = state.panel.open ? "open" : "closed";
  // The handles say where the drawer is, never what is in it: an empty drawer still opens, so the chevron stays
  // until a session offers the panel at least once.
  at("panel-toggle").hidden = !state.panel.tab && !state.panel.open;
  const panel = at("panel");
  if (!state.panel.open) {
    at("panel-name").textContent = "";
    panel.dataset.owner = "";
    vacate();
    return;
  }
  const plugin = plugins.get(state.panel.tab);
  if (!plugin) return;
  panel.dataset.owner = plugin.id;
  at("panel-name").textContent = plugin.label || plugin.id;
  // A change of owner lays the new template down and writes it once; the same owner is left as it stands.
  if (standing !== plugin.id) {
    const body = clear(at("panel-body"));
    mounted = plugin.frame();
    body.append(mounted);
    standing = plugin.id;
    plugin.fill(mounted, state.session);
    const asked = plugin.load && plugin.load(state.session);
    if (asked && asked.then) {
      asked.then(() => refresh(plugin.id)).catch(() => {});
    }
  } else if (mounted) {
    plugin.fill(mounted, state.session);
  }
}
