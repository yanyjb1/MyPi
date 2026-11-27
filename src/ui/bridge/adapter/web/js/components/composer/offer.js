// What one word offers: a command line's own names and argument kinds, or a plain word's paths. The popover over the
// composer is `path.js`'s business; this decides what goes in it.

import { listing, rungs } from "../../core/api.js";
import { at } from "../../core/template.js";
import { cwd, state } from "../../core/store.js";
import { before, word } from "./say.js";
import { argOf, table } from "./summon.js";

// How many candidates one level shows at most.
export const MAX = 8;

// A word shaped like a path: anything holding a slash except the bare root.
function shaped(said) {
  return said.includes("/") && said !== "/";
}

// The index of the word being typed on a line split by whitespace; 0 is the command name.
function wordIndex(said) {
  const words = said.split(/\s+/).filter(Boolean).length;
  return /\s$/.test(said) ? words : Math.max(words - 1, 0);
}

// How the typed text hit a name: a prefix first, then the letters in order, then in any order.
function hit(typed, name, dots) {
  const low = name.toLowerCase();
  const want = typed.toLowerCase();
  if (dots && low.startsWith(".") && !want.startsWith(".") && low.slice(1).startsWith(want)) return 0;
  if (low.startsWith(want)) return 0;
  const chars = [...low];
  let cursor = 0;
  for (const one of want) {
    const found = chars.indexOf(one, cursor);
    if (found < 0) {
      cursor = -1;
      break;
    }
    cursor = found + 1;
  }
  if (cursor >= 0) return 1;
  const bag = [...chars];
  for (const one of want) {
    const found = bag.indexOf(one);
    if (found < 0) return null;
    bag.splice(found, 1);
  }
  return 2;
}

// Paths under the word's own head; `forced` is an argument position that wants nothing but a path.
async function paths(said, forced) {
  if (!forced && !shaped(said)) return [];
  const cut = said.lastIndexOf("/");
  const head = cut < 0 ? "" : said.slice(0, cut + 1);
  const prefix = said.slice(head.length);
  const answer = await listing(cwd(), said);
  const scored = [];
  for (const entry of answer.entries || []) {
    const rank = hit(prefix, entry.name, true);
    if (rank === null) continue;
    scored.push({ entry, rank });
  }
  scored.sort(
    (a, b) => a.rank - b.rank || Number(b.entry.dir) - Number(a.entry.dir) || a.entry.name.localeCompare(b.entry.name),
  );
  return scored.slice(0, MAX).map(({ entry }) => ({
    kind: "path",
    name: entry.name + (entry.dir ? "/" : ""),
    detail: entry.dir ? "dir" : "file",
    dir: entry.dir,
    insert: head + entry.name + (entry.dir ? "/" : ""),
  }));
}

// The models the server offers, as the switch and model words take them.
function models(said) {
  const list = (state.pools && state.pools.models) || [];
  return list
    .map((model) => ({ model, rank: hit(said, model.address, false) }))
    .filter((one) => one.rank !== null)
    .sort((a, b) => a.rank - b.rank)
    .slice(0, MAX)
    .map(({ model }) => ({
      kind: "name",
      name: model.address,
      detail: model.name || "",
      dir: false,
      insert: model.address,
    }));
}

// The rungs this model declares, `auto` first.
function efforts(said) {
  return rungs()
    .map((rung) => ({ rung, rank: hit(said, rung, false) }))
    .filter((one) => one.rank !== null)
    .sort((a, b) => a.rank - b.rank)
    .slice(0, MAX)
    .map(({ rung }) => ({
      kind: "name",
      name: rung,
      detail: "",
      dir: false,
      insert: rung,
    }));
}

// The words this page answers; one that takes arguments gets a trailing space so the cursor lands on them.
function commands(said) {
  const out = [];
  for (const one of table) {
    if (hit(said, one.name, false) === null) continue;
    out.push({
      kind: "command",
      name: "/" + one.name,
      detail: one.args.join(" "),
      dir: false,
      insert: "/" + one.name + (one.args.length ? " " : ""),
    });
  }
  return out.slice(0, MAX);
}

// What the composer's word under the cursor offers: a command line completes its own vocabulary, and any other line
// completes paths — never both.
export async function offer() {
  const box = at("say");
  const said = before(box);
  const typed = word(box).text;
  if (said.includes("\n") || !said.trimStart().startsWith("/")) {
    return paths(typed, false);
  }
  const index = wordIndex(said);
  // The word carries the leading slash; the table's names do not.
  if (index === 0) return commands(typed.replace(/^\//, ""));
  const name = said.trim().slice(1).split(/\s+/)[0];
  const kind = argOf(name, index - 1);
  if (kind === "path") return paths(typed, true);
  if (kind === "model") return models(typed);
  if (kind === "effort") return efforts(typed);
  return [];
}

// The directory field is one path argument and nothing else.
export function directory(text) {
  return paths(text, true);
}
