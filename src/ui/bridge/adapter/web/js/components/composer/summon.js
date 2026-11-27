// The slash words the composer answers: each one runs inside the page, never leaves as text, and declares the
// arguments its own position completes.

import { fresh } from "../chat/list.js";
import {
  compact,
  defaultModel,
  interrupt,
  move,
  pools,
  refresh,
  rename,
  setRung,
  switchModel,
} from "../../core/api.js";
import { give as openPanel } from "../panels.js";

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

// `provider:model`, safe split at the first colon only.
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
word("stop", [], () => interrupt());
word("sessions", [], () => refresh());
word("pools", [], () => pools());

// The one panel: opening it is the same act as asking it for its rows, and it is the only one there is.
word("usage", [], () => openPanel("usage"));

export { table };