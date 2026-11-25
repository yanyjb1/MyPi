// The left column: the session list, the one act it offers — a draft — and the delete each row carries.

import { at, clear, el } from "../base/dom.js";
import { state } from "../base/state.js";
import { refresh } from "../base/read.js";
import { enter } from "../chat/history/enter.js";
import { remove } from "../base/write.js";
import { fresh } from "./draft.js";
import { confirm, typed } from "./ask.js";
import { rename } from "../base/write.js";
import { short } from "../base/note.js";

function title(row) {
  if (row.name) return row.name;
  if (row.first_user) return short(row.first_user, 80);
  return "session " + row.id;
}

export function draw() {
  const list = clear(at("session-list"));
  if (!state.rows.length) {
    list.append(el("li", { class: "empty", text: "no sessions yet" }));
    return;
  }
  for (const row of state.rows) {
    const running = state.session && state.session.id === row.id &&
      state.session.busy;
    list.append(
      el(
        "li",
        {
          "data-id": String(row.id),
          "data-on": row.id === state.active ? "1" : "0",
          "data-live": running ? "1" : "0",
          onclick: () => enter(row.id),
        },
        el("span", { class: "title", text: title(row) }),
        el(
          "span",
          { class: "meta" },
          el("span", { text: "#" + row.id }),
          row.rounds ? el("span", { text: row.rounds + "r" }) : null,
          row.last_ts
            ? el("span", {
              text: short(String(row.last_ts).replace("T", " "), 16),
            })
            : null,
          running ? el("span", { text: "running" }) : null,
        ),
        el(
          "span",
          { class: "acts" },
          el(
            "button",
            {
              class: "icon rename",
              title: "rename this session",
              "aria-label": "rename this session",
              onclick: (event) => {
                event.stopPropagation();
                ask(row.id, row.name);
              },
            },
            el(
              "svg",
              { viewBox: "0 0 16 16", "aria-hidden": "true" },
              el("path", { d: "M10.6 2.4l3 3L6 13l-3.6.7.7-3.6z" }),
            ),
          ),
          el(
            "button",
            {
              class: "icon remove",
              title: "delete this session",
              "aria-label": "delete this session",
              onclick: async (event) => {
                event.stopPropagation();
                if (
                  await confirm(
                    "delete session #" + row.id + " for good?",
                    "delete",
                  )
                ) remove(row.id);
              },
            },
            el(
              "svg",
              { viewBox: "0 0 16 16", "aria-hidden": "true" },
              el("path", {
                d: "M2.5 4.5h11M6.5 4.5V3h3v1.5M4.2 4.5l.6 9h6.4l.6-9M6.8 7v4M9.2 7v4",
              }),
            ),
          ),
        ),
      ),
    );
  }
}

// A new name, or nothing at all: the native modal is the only place a row is renamed.
async function ask(id, name) {
  const said = await typed("name this session", "rename", name || "");
  if (said) rename(said, id);
}

function start() {
  fresh();
  at("say").focus();
}

export function wire() {
  at("new-session").addEventListener("click", start);
  // The list goes stale quietly, so it is asked again while the tab is in front.
  setInterval(() => {
    if (document.visibilityState === "visible") refresh();
  }, 15_000);
}
