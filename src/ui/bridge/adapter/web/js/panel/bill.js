// The usage drawer's parts: the four figures, where the tokens went, one card a model, the session's foot.

import { el } from "../base/dom.js";
import { money, tokens } from "../base/note.js";
import { contextWindow as contextSize } from "../base/model.js";

// A part of a whole as the percentage its bar is drawn at.
function share(part, whole) {
  if (!whole) return 0;
  return Math.min(100, Math.max(0, (part / whole) * 100));
}

// One key of the rows, summed.
function sum(rows, key) {
  return rows.reduce((all, row) => all + row[key], 0);
}

// One figure of the head: its word, its figure under it, and the bar or the note that explains it.
function cell(word, value, note, tone, bar) {
  const box = el(
    "div",
    {},
    el("span", { class: "k", text: word }),
    el("span", { class: "v " + tone }, value),
  );
  if (bar) {
    const fill = el("i", { class: tone });
    fill.style.width = share(bar[0], bar[1]) + "%";
    box.append(el("div", { class: "bill-track" }, fill));
  }
  if (note) box.append(el("span", { class: "f", text: note }));
  return box;
}

// The four figures the drawer opens with: what the session cost, how many tokens it took, what the cache spared
// of them, and where the context stands against its window.
export function head(bill, rows) {
  const count = sum(rows, "total");
  const rounds = sum(rows, "rounds");
  const cached = sum(rows, "cached");
  const room = contextSize();
  const each = bill.cny && rounds ? money(bill.cny / rounds) + " / round" : "";
  const of = room ? el("small", { text: " / " + tokens(room) }) : null;
  const mark = bill.estimated ? "~" : "";
  return el(
    "div",
    { class: "bill-kpi" },
    cell("spend", bill.cny ? money(bill.cny) : "—", each, "t-warn"),
    cell("tokens", tokens(count), count.toLocaleString("en-US"), ""),
    cell(
      "cache hit",
      (count ? (cached / count) * 100 : 0).toFixed(2) + "%",
      "",
      "t-accent",
      [cached, count],
    ),
    cell(
      "context",
      [mark + tokens(bill.context), of],
      "",
      "t-success",
      [bill.context, room],
    ),
  );
}

// Where the tokens went: one bar of the four parts, and the legend naming each with its count.
export function split(rows) {
  const total = sum(rows, "total");
  const parts = [
    ["cache read", sum(rows, "cached"), "t-accent"],
    ["input", sum(rows, "input"), "t-warn"],
    ["output", sum(rows, "completion"), "t-success"],
    ["cache write", sum(rows, "written"), "t-dim"],
  ];
  const bar = el("div", { class: "bill-bar" });
  const legend = el("div", { class: "bill-legend" });
  for (const [word, count, tone] of parts) {
    const slice = el("i", { class: tone });
    slice.style.width = share(count, total) + "%";
    bar.append(slice);
    legend.append(
      el(
        "div",
        {},
        el("span", { class: "n" }, el("i", { class: "swatch " + tone }), word),
        el("b", { text: tokens(count) }),
      ),
    );
  }
  return el("div", { class: "bill-split" }, bar, legend);
}

// The five numbers usage.rs bills, in the order they partition a row.
function parts(row) {
  return [
    ["total", row.total, ""],
    ["input", row.input, ""],
    ["output", row.completion, ""],
    ["read", row.cached, "t-accent"],
    ["write", row.written, "t-dim"],
  ];
}

// One model's card: who it was, what it cost, its share of the table, then its numbers with their own slices.
export function card(row, whole, sub) {
  const nums = el("div", { class: "nums" });
  for (const [word, count, tone] of parts(row)) {
    nums.append(
      el(
        "div",
        {},
        el("span", { class: "k", text: word }),
        el("span", { class: "v", text: tokens(count) }),
        el("span", {
          class: "p " + tone,
          text: share(count, row.total).toFixed(1) + "%",
        }),
      ),
    );
  }
  const top = el(
    "div",
    { class: "top" },
    el("span", { class: "name", text: row.address || "—" }),
    sub ? el("span", { class: "tag", text: "sub" }) : null,
    el(
      "span",
      { class: "cost", text: row.price ? money(row.price.cny) : "—" },
      el("small", { text: share(row.total, whole).toFixed(1) + "%" }),
    ),
  );
  return el(
    "div",
    { class: "bill-card" + (sub ? " sub" : ""), title: row.address || "" },
    top,
    nums,
  );
}

// Every model's card, the long tail behind one bar until it is asked for, the subagents' pool under its head.
export function cards(rows, pool, opened, ask) {
  const whole = sum(rows, "total");
  const held = sum(pool, "total");
  const box = el("div", { class: "bill-cards" });
  const shown = 3;
  for (const row of rows.slice(0, shown)) box.append(card(row, whole, false));
  const tail = rows.slice(shown);
  if (opened) { for (const row of tail) box.append(card(row, whole, false)); }
  if (tail.length) {
    box.append(
      el("button", {
        class: "bill-more",
        type: "button",
        text: opened ? "− fewer" : "+ " + tail.length + " more",
        onclick: ask,
      }),
    );
  }
  if (pool.some((row) => row.total)) {
    box.append(el("h2", { text: "subagents" }));
    for (const row of pool) box.append(card(row, held, true));
  }
  return box;
}

// The session's own foot: what the two sides cost, and what a round carried.
export function foot(bill, rows) {
  const total = sum(rows, "total");
  const rounds = sum(rows, "rounds");
  const sub = bill.subagent_price;
  return el(
    "div",
    { class: "bill-foot" },
    line(
      "main / subagents",
      (bill.cny ? money(bill.cny) : "—") +
        " / " + (sub ? money(sub.cny) : "—"),
    ),
    line("tokens a round", rounds ? tokens(Math.round(total / rounds)) : "—"),
  );
}

// One line of the foot: its word on the left, its figure right-aligned against the drawer's edge.
function line(word, value) {
  return el(
    "div",
    { class: "bill-row" },
    el("span", { class: "l", text: word }),
    el("span", { class: "r", text: value }),
  );
}
