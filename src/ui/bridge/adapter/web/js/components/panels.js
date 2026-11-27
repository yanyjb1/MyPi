// The right drawer: one owner at a time, its own template, and only its own writes on top of it. A panel brings its
// own sheet and its own markup, so it is added or taken away without the page knowing what is inside it.

import { at, clear, fetchTemplate, stylesheet } from "../core/template.js";
import { closePanel, openPanel, state } from "../core/store.js";

const plugins = new Map();
// Whose template stands in the drawer's body; empty when nobody's does.
let standing = null;
// Whose template is on its way, so a burst of redraws asks for it once.
let laying = null;
// The node that owner laid down, which every fill of it writes into.
let mounted = null;

export const PanelRegistry = {
  register(plugin) {
    if (!plugin || !plugin.id) return;
    if (typeof plugin.template !== "function" || typeof plugin.fill !== "function") {
      return;
    }
    plugins.set(plugin.id, plugin);
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
    laying = null;
    mounted = null;
  }
}

// Laying a panel down is two acts — its sheet in the head, its markup in the body — and then its first fill.
async function lay(plugin) {
  laying = plugin.id;
  if (plugin.css) stylesheet(plugin.css);
  const fragment = await fetchTemplate(plugin.template);
  // Another owner took the drawer, or this one was shut, while the markup was on its way.
  if (laying !== plugin.id || state.panel.tab !== plugin.id) return;
  laying = null;
  const body = clear(at("panel-body"));
  mounted = fragment.firstElementChild;
  body.append(fragment);
  standing = plugin.id;
  if (plugin.load) await plugin.load(state.session);
  if (standing === plugin.id) plugin.fill(mounted, state.session);
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
  if (standing !== plugin.id && laying !== plugin.id) {
    lay(plugin).catch(() => {});
  } else if (mounted && standing === plugin.id) {
    plugin.fill(mounted, state.session);
  }
}