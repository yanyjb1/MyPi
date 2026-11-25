// One card of the transcript: a thinking block, a tool call, and the one line a notice takes.

import { el } from "../../base/dom.js";
import { body } from "./mark.js";
import { elapsed, note, resultText } from "../../base/note.js";
import { state } from "../../base/state.js";

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

// Thinking block, empty: the card a growing thought paints into.
export function reasoningNode() {
  const card = el("details", {
    class: "think",
    open: state.streaming ? "" : null,
  });
  const summary = el(
    "summary",
    { class: "think-head" },
    el("span", { class: "label", text: "thinking" }),
  );
  const content = el("div", { class: "body md think-body" });
  card.append(summary, content);
  return card;
}

// Thinking block: rendered in muted gray prose, collapsed to a subtle indicator by default.
export function reasoning(text) {
  const card = reasoningNode();
  card.querySelector(".think-body").innerHTML = body(text || "");
  return card;
}

// One argument's value as the reader should see it: strings stand as they are, a string that parses as JSON is
// laid out as JSON, and objects and lists are laid out whole rather than as their `String()`.
function shown(value) {
  if (typeof value === "string") {
    const text = value.trim();
    if (text.startsWith("{") || text.startsWith("[")) {
      try {
        return JSON.stringify(JSON.parse(text), null, 2);
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

// Tool call: two-part layout (head with {name}: {intent} and toggle, collapsible body with req args and res output).
export function tool(name, args, intent, status, result, took, progress) {
  const card = el("div", {
    class: "tool-card",
    "data-state": status || "running",
  });

  const toolName = name || "tool";
  const desc = intent || progress || note(name, args, "");
  const head = el(
    "div",
    { class: "tool-head" },
    el("span", { class: "dot", text: "●" }),
    el("span", { class: "tool-name", text: toolName }),
    desc ? el("span", { class: "tool-intent", text: ": " + desc }) : null,
    el(
      "span",
      { class: "tool-meta" },
      took ? el("span", { class: "took", text: elapsed(took) }) : null,
      el("span", {
        class: "state-tag",
        text: status === "running"
          ? "running"
          : status === "failed"
          ? "err"
          : "ok",
      }),
      el("button", {
        class: "icon tool-toggle",
        title: "toggle details",
        "aria-label": "toggle details",
        text: "▾",
      }),
    ),
  );

  const detail = el("div", { class: "tool-detail", hidden: true });

  // 1. Head/Request: arguments. Whole, and never shortened: a reader who opened the card wants the value that
  //    was sent, not the first two hundred characters of it. A string that is itself JSON is laid out as JSON.
  const pairs = args && typeof args === "object" ? Object.entries(args) : [];
  if (pairs.length) {
    const reqBox = el("div", { class: "tool-part req" });
    reqBox.append(
      el("div", { class: "part-label", text: "args" }),
      el(
        "div",
        { class: "args" },
        pairs.map(([k, v]) => [
          el("span", { class: "k", text: k }),
          el("span", { class: "v", text: shown(v) }),
        ]),
      ),
    );
    detail.append(reqBox);
  }

  // 2. Tail/Result: output
  const said = result ? resultText(result) : "";
  if (said) {
    const resBox = el("div", { class: "tool-part res" });
    resBox.append(
      el("div", { class: "part-label", text: "result" }),
      el("pre", {
        class: "out",
        "data-tone": status === "failed" ? "failed" : null,
        text: said,
      }),
    );
    detail.append(resBox);
  }

  head.addEventListener("click", () => {
    detail.hidden = !detail.hidden;
    head.querySelector(".tool-toggle").textContent = detail.hidden ? "▸" : "▾";
  });

  card.append(head, detail);
  return card;
}

export function noteLine(level, source, text) {
  return el(
    "div",
    {
      class: "note",
      "data-level": level === "error"
        ? "error"
        : level === "warning"
        ? "warn"
        : null,
    },
    el("span", { class: "src", text: source }),
    el("span", { text: text }),
  );
}

export function wake(text) {
  return noteLine("notice", "wake", text || "");
}
