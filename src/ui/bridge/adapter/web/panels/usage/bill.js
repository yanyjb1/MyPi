// One model's share of the bill, and every card the drawer prints from it: the four figures at its head, where the
// tokens went, one card a model, and the session's foot. The columns are those of `usage.rs`.

import { build, text } from "../../js/core/template.js";
import { money, tokens } from "../../js/core/format.js";
import { contextWindow } from "../../js/core/api.js";

// A part of a whole, as the percentage its bar is drawn at.
function share(part, whole) {
  if (!whole) return 0;
  return Math.min(100, Math.max(0, (part / whole) * 100));
}

// One key of the rows, summed.
function sum(rows, key) {
  return rows.reduce((all, row) => all + row[key], 0);
}

// One model's share of the bill, decoded from a pool group the way `usage.rs` decodes it.
export function row(group) {
  const prompt = Number(group.prompt_tokens || 0);
  const completion = Number(group.completion_tokens || 0);
  const cached = Number(group.cached_tokens || 0);
  const written = Number(group.cache_write_tokens || 0);
  return {
    address: group.address || group.model || "",
    rounds: Number(group.rounds || 0),
    prompt,
    completion,
    cached,
    written,
    total: prompt + completion,
    // Input tokens no cache reported: the five numbers are one partition of the total.
    input: Math.max(0, prompt - cached - written),
    price: group.price || null,
  };
}

// One figure of the head: its word, its figure under it, and the bar or the note that explains it.
function cell(word, value, note, tone, bar) {
  const box = build("tpl-bill-cell");
  text(box, "k", word);
  const figure = box.querySelector('[data-ref="v"]');
  figure.textContent = value;
  if (tone) figure.classList.add(tone);
  if (bar) {
    const track = box.querySelector('[data-ref="track"]');
    track.hidden = false;
    const fill = track.firstElementChild;
    if (tone) fill.classList.add(tone);
    fill.style.width = share(bar[0], bar[1]) + "%";
  }
  text(box, "f", note);
  return box;
}

// The four figures the drawer opens with: what the session cost, how many tokens it took, what the cache spared of
// them, and where the context stands against its window.
export function head(bill, rows) {
  const count = sum(rows, "total");
  const rounds = sum(rows, "rounds");
  const cached = sum(rows, "cached");
  const room = contextWindow();
  const each = bill.cny && rounds ? money(bill.cny / rounds) + " / round" : "";
  const mark = bill.estimated ? "~" : "";
  const box = build("tpl-bill-head");
  box.append(
    cell("spend", bill.cny ? money(bill.cny) : "—", each, "t-warn"),
    cell("tokens", tokens(count), count.toLocaleString("en-US"), ""),
    cell("cache hit", (count ? (cached / count) * 100 : 0).toFixed(2) + "%", "", "t-accent", [cached, count]),
    cell("context", mark + tokens(bill.context) + (room ? " / " + tokens(room) : ""), "", "t-success", [
      bill.context,
      room,
    ]),
  );
  return box;
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
  const box = build("tpl-bill-split");
  const bar = box.querySelector('[data-ref="bar"]');
  const legend = box.querySelector('[data-ref="legend"]');
  for (const [word, count, tone] of parts) {
    const slice = document.createElement("i");
    slice.className = tone;
    slice.style.width = share(count, total) + "%";
    bar.append(slice);
    const line = build("tpl-bill-legend");
    text(line, "word", word);
    line.querySelector(".swatch").classList.add(tone);
    text(line, "count", tokens(count));
    legend.append(line);
  }
  return box;
}

// The five numbers `usage.rs` bills, in the order they partition a row.
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
  const box = build("tpl-bill-card");
  if (sub) box.classList.add("sub");
  box.title = row.address || "";
  text(box, "name", row.address || "—");
  text(box, "tag", sub ? "sub" : "");
  text(box, "cost", row.price ? money(row.price.cny) : "—");
  text(box, "share", share(row.total, whole).toFixed(1) + "%");
  const nums = box.querySelector('[data-ref="nums"]');
  for (const [word, count, tone] of parts(row)) {
    const one = build("tpl-bill-num");
    text(one, "k", word);
    text(one, "v", tokens(count));
    const shareage = one.querySelector('[data-ref="p"]');
    shareage.textContent = share(count, row.total).toFixed(1) + "%";
    if (tone) shareage.classList.add(tone);
    nums.append(one);
  }
  return box;
}

// Every model's card, the long tail behind one bar until it is asked for, the subagents' pool under its head.
export function cards(rows, pool, opened, ask) {
  const whole = sum(rows, "total");
  const held = sum(pool, "total");
  const box = build("tpl-bill-cards");
  const shown = 3;
  for (const one of rows.slice(0, shown)) box.append(card(one, whole, false));
  const tail = rows.slice(shown);
  if (opened) {
    for (const one of tail) box.append(card(one, whole, false));
  }
  if (tail.length) {
    const more = build("tpl-bill-more");
    more.textContent = opened ? "− fewer" : "+ " + tail.length + " more";
    more.addEventListener("click", ask);
    box.append(more);
  }
  if (pool.some((one) => one.total)) {
    const title = document.createElement("h2");
    title.textContent = "subagents";
    box.append(title);
    for (const one of pool) box.append(card(one, held, true));
  }
  return box;
}

// One line of the foot: its word on the left, its figure right-aligned against the drawer's edge.
function line(word, value) {
  const node = build("tpl-bill-row");
  text(node, "l", word);
  text(node, "r", value);
  return node;
}

// The session's own foot: what the two sides cost, and what a round carried.
export function foot(bill, rows) {
  const total = sum(rows, "total");
  const rounds = sum(rows, "rounds");
  const sub = bill.subagent_price;
  const box = build("tpl-bill-foot");
  box.append(
    line("main / subagents", (bill.cny ? money(bill.cny) : "—") + " / " + (sub ? money(sub.cny) : "—")),
    line("tokens a round", rounds ? tokens(Math.round(total / rounds)) : "—"),
  );
  return box;
}
