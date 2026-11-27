// The citation drawer: the pages one round's prose cites, one card per address. A pill is the only way in, so the list
// is read from the pills the reader can see rather than guessed again from the text behind them.

import { citeTitles } from "../../js/core/api.js";
import { state } from "../../js/core/store.js";
import { at, build, fetchTemplate, ref, text } from "../../js/core/template.js";
import { give } from "../../js/components/panels.js";

const HERE = "/panels/citation/";

// The addresses a site may answer its icon at, tried in this order; the first that loads is the one the card wears.
const ICONS = ["/favicon.ico", "/icon.svg", "/favicon.png", "/apple-touch-icon.png"];

// The round whose pages stand here, and the address a press named.
let want = null;
// What each address was found to call itself; a page that would not say holds nothing.
const titles = new Map();
// Whether the drawer is already waiting on the door for the addresses it still lacks.
let asking = false;
// What is drawn, so a redraw of the same thing leaves the cards standing.
let stamp = "";
// The node the host laid down.
let box = null;

// The round a pressed pill stands in: a landed round says its own, and the running round has no turn of its own yet.
function turnOf(pill) {
  const turn = pill.closest(".turn");
  if (!turn) return null;
  return turn.dataset.turn ? Number(turn.dataset.turn) : state.session.liveTurn;
}

// The node a round stands as, whether it has landed or is still being said.
function turnNode(turn) {
  const sheet = at("sheet");
  if (!sheet) return null;
  if (turn == null) return sheet.querySelector(".turn.live");
  return sheet.querySelector('[data-turn="' + turn + '"]');
}

// Every pill that round shows, one entry per address: the same page cited twice is one card, under its first number.
function pillsOf(node) {
  const found = new Map();
  if (!node) return [];
  for (const pill of node.querySelectorAll(".cite-pill")) {
    const url = pill.dataset.url || "";
    if (!/^https?:\/\//.test(url)) continue;
    const num = Number(pill.dataset.cite);
    if (!found.has(url) || num < found.get(url)) found.set(url, num);
  }
  return [...found].map(([url, num]) => ({ url, num })).sort((a, b) => a.num - b.num);
}

// The newest round worth standing open on, for a drawer asked for by name rather than by a pill.
function newestTurn() {
  const held = state.session;
  if (!held) return null;
  if (held.live.length) return held.liveTurn;
  const last = held.structure[held.structure.length - 1];
  return last ? last.turn : null;
}

// One pill anywhere in the transcript: its round's pages are what the drawer then holds.
function pressed(event) {
  const pill = event.target.closest(".cite-pill");
  if (!pill) return;
  if (!state.session) return;
  want = { turn: turnOf(pill), url: pill.dataset.url };
  stamp = "";
  give("citation");
}

// One address as a card speaks it: the title the page gave, else the words its own path is made of, else the domain,
// which is then as loud as a title would have been.
function resolve(url, title) {
  let parsed;
  try {
    parsed = new URL(url);
  } catch {
    return { title: url, url, domain: "", mode: "domain" };
  }
  const domain = parsed.hostname.replace(/^www\./, "");
  const short = domain + (parsed.pathname === "/" ? "" : parsed.pathname) + parsed.search;
  if (title) return { title, url: short, domain, mode: "title" };
  const last = parsed.pathname.split("/").filter(Boolean).pop() || "";
  const plain = last.replace(/\.(html?|md|php|aspx?)$/i, "");
  if (/[-_]/.test(plain)) {
    try {
      const words = decodeURIComponent(plain).replace(/[-_]+/g, " ").trim();
      if (words) return { title: words, url: short, domain, mode: "slug" };
    } catch {
      // A path that will not decode named nothing readable, and speaks by its domain like any other.
    }
  }
  return { title: domain || url, url: short, domain, mode: "domain" };
}

// The icon over its own letter: the first address that answers a picture, and the letter when none does.
function icon(node, domain) {
  const img = ref(node, "img");
  ref(node, "initial").textContent = (domain[0] || "?").toUpperCase();
  img.hidden = true;
  let at = 0;
  const next = () => {
    if (at >= ICONS.length) return;
    img.src = "https://" + domain + ICONS[at];
    at += 1;
  };
  img.addEventListener("load", () => {
    img.hidden = false;
  });
  img.addEventListener("error", () => {
    img.hidden = true;
    next();
  });
  next();
}

function card(one) {
  const node = build("tpl-cite-card");
  const said = resolve(one.url, titles.get(one.url));
  node.dataset.active = one.url === (want && want.url) ? "1" : "0";
  node.dataset.fallback = said.mode;
  node.title = one.url;
  text(node, "num", one.num);
  text(node, "title", said.title);
  text(node, "url", said.url);
  icon(node, said.domain);
  node.addEventListener("click", () => {
    window.open(one.url, "_blank", "noopener");
  });
  return node;
}

function paint() {
  if (!box) return;
  const cites = pillsOf(turnNode(want && want.turn));
  const said = [
    want ? want.url : "",
    cites.map((one) => one.num + " " + one.url).join("|"),
    cites.map((one) => titles.get(one.url) || "").join("|"),
  ].join("#");
  if (said === stamp) return;
  stamp = said;
  const list = document.createElement("div");
  list.className = "cite-list";
  for (const one of cites) list.append(card(one));
  if (!cites.length) {
    const empty = document.createElement("div");
    empty.className = "empty";
    empty.textContent = "this round cites nothing";
    list.append(empty);
  }
  box.replaceChildren(list);
  ask(cites);
}

// Every address the cards show a title for, asked of the door once: it reads the page, which a page cannot do.
function ask(cites) {
  const need = cites.map((one) => one.url).filter((url) => !titles.has(url));
  if (!need.length || asking) return;
  asking = true;
  citeTitles(need)
    .then((answer) => {
      const got = (answer && answer.titles) || {};
      for (const url of need) titles.set(url, got[url] || null);
    })
    .catch(() => {})
    .finally(() => {
      for (const url of need) if (!titles.has(url)) titles.set(url, null);
      asking = false;
      stamp = "";
      paint();
    });
}

export function wire() {
  at("sheet").addEventListener("click", pressed);
}

export function register(PanelRegistry) {
  PanelRegistry.register({
    id: "citation",
    label: "citations",
    css: HERE + "citation.css",
    template: () => fetchTemplate(HERE + "citation.html"),
    fill(node) {
      box = node;
      if (!state.session) return;
      if (!want) want = { turn: newestTurn(), url: "" };
      paint();
    },
  });
}
