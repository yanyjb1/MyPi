// The one card a tool call takes: a head that says what was asked, and a detail that says what came back. The card
// is a template; the shape of one tool's detail is the registry's business, and the fallback is all of them.

import { build, parts, text } from "../core/template.js";
import { elapsed, note, resultText } from "../core/format.js";

const shapes = new Map();

// A tool that draws its own detail registers here and gets the card it is given; a name nobody claimed takes the
// fallback, which is every argument and the whole result.
export function register(name, draw) {
  shapes.set(name, draw);
}

// A tool call's arguments as the card shows them: the object it stands for, or the text it arrived as.
export function parsed(raw) {
  if (!raw) return {};
  if (typeof raw === "object") return raw;
  try {
    return JSON.parse(raw);
  } catch {
    return { raw: String(raw) };
  }
}

// One argument's value as the reader should see it: strings stand as they are, a string that parses as JSON is laid
// out as JSON, and objects and lists are laid out whole rather than as their `String()`.
function shown(value) {
  if (typeof value === "string") {
    const said = value.trim();
    if (said.startsWith("{") || said.startsWith("[")) {
      try {
        return JSON.stringify(JSON.parse(said), null, 2);
      } catch {
        return value;
      }
    }
    return value;
  }
  if (value == null) return String(value);
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

// The arguments, whole: a reader who opened the card wants the value that was sent, not its first two hundred
// characters. A string that is itself JSON is laid out as JSON.
function asked(detail, args) {
  const pairs = args && typeof args === "object" ? Object.entries(args) : [];
  if (!pairs.length) return;
  const part = build("tpl-tool-part");
  text(part, "label", "args");
  const grid = part.querySelector('[data-ref="args"]');
  for (const [k, v] of pairs) {
    const [key, value] = parts("tpl-tool-arg");
    key.textContent = k;
    value.textContent = shown(v);
    grid.append(key, value);
  }
  part.querySelector('[data-ref="out"]').remove();
  detail.append(part);
}

// The result, the length the tool gave it: the card scrolls inside itself and nothing is cut out of it.
function answered(detail, said, tone) {
  if (!said) return;
  const part = build("tpl-tool-part");
  text(part, "label", "result");
  part.querySelector('[data-ref="args"]').remove();
  const out = part.querySelector('[data-ref="out"]');
  out.textContent = said;
  if (tone) out.dataset.tone = tone;
  detail.append(part);
}

// A card is the same wherever it stands: laid once, folded until it is clicked, and opened again by a click.
export function card(name, args, intent, status, result, took, progress) {
  const node = build("tpl-tool");
  node.dataset.state = status || "running";
  text(node, "name", name || "tool");
  // The model's own word for the call, and what it asked when it said nothing.
  const line = intent || progress || note(name, args, "");
  text(node, "intent", line ? ": " + line : "");
  text(node, "took", took ? elapsed(took) : "");
  text(
    node,
    "state",
    status === "running" ? "running" : status === "failed" ? "err" : "ok",
  );

  const detail = node.querySelector('[data-ref="detail"]');
  const draw = shapes.get(name);
  if (draw) draw(detail, parsed(args), result || "");
  else {
    asked(detail, parsed(args));
    answered(
      detail,
      result ? resultText(result) : "",
      status === "failed" ? "failed" : null,
    );
  }

  const head = node.querySelector(".tool-head");
  const toggle = node.querySelector(".tool-toggle");
  head.addEventListener("click", () => {
    detail.hidden = !detail.hidden;
    toggle.textContent = detail.hidden ? "▸" : "▾";
  });
  return node;
}