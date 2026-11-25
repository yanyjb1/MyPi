// The slash words the composer answers: each one runs inside the page, never leaves as text, and
// declares the arguments its own position completes.

import { announce, closePanel, state } from "../../base/state.js";
import { fresh } from "../../rail/draft.js";
import {
  compact,
  defaultModel,
  interrupt,
  move,
  rename,
  setRung,
  switchModel,
} from "../../base/write.js";
import { pools, refresh } from "../../base/read.js";
import { at } from "../../base/dom.js";
import { give as openPanel } from "../../panel/panels.js";

const words = new Map();
const table = [];

// One word: the name it answers to, the kind each argument position completes, and what it does.
function word(name, args, run) {
  table.push({ name, args });
  words.set(name, run);
}

// The argument kind the cursor sits on; `null` is past the last one.
export function argOf(name, level) {
  const found = table.find((one) => one.name === name);
  if (!found) return null;
  return found.args[level] ?? null;
}

// `provider:model`, safe split at first colon only
function pair(said) {
  const str = String(said || "").trim();
  const at = str.indexOf(":");
  if (at <= 0 || at === str.length - 1) return null;
  return { provider: str.slice(0, at), model: str.slice(at + 1) };
}

// A line of the composer: the word it names, and whether it was one.
export function summon(line) {
  const text = String(line || "").trim();
  if (!text.startsWith("/")) return false;
  const [head, ...rest] = text.slice(1).split(/\s+/);
  const run = words.get(head);
  if (!run) return false;
  run(rest.join(" ").trim());
  return true;
}

word("new", [], () => fresh());
word("name", ["text"], (said) => said && rename(said));
word("cwd", ["path"], (said) => said && move(said));
word("effort", ["effort"], (said) => said && setRung(said));
word("switch", ["model"], (said) => {
  const found = pair(said);
  if (found) switchModel(found.provider + ":" + found.model);
});
word("model", ["model"], (said) => {
  const found = pair(said);
  if (found) defaultModel(found.provider + ":" + found.model);
  else pools();
});
word("compact", ["text"], (said) => compact(said));
word("rewind", [], () => openPanel("tree"));
word("stop", [], () => interrupt());
word("sessions", [], () => refresh());
word("pools", [], () => pools());

// The panel words: opening one is the same act as asking it for its rows, and it is the only one open.
word("usage", [], () => openPanel("usage"));
word("todo", [], () => openPanel("todo"));
word("memo", [], () => openPanel("memo"));
word("tree", [], () => openPanel("tree"));
word("child", [], () => openPanel("child"));
word("children", [], () => openPanel("child"));
word("setup", [], () => openPanel("setup"));

export { announce, at, closePanel, table };
