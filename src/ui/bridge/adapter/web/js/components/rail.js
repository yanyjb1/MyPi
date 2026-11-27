// The left column: the session list, the one act it offers — a draft — and the two writes each row carries, both
// asked for in a native modal so neither is a slip.

import { at, build, clear, text } from "../core/template.js";
import { state } from "../core/store.js";
import { refresh, remove, rename } from "../core/api.js";
import { enter, fresh } from "./chat/list.js";
import { short } from "../core/format.js";

function title(row) {
  if (row.name) return row.name;
  if (row.first_user) return short(row.first_user, 80);
  return "session " + row.id;
}

// A modal that answers with its own field's text, or with `true` when it only had to be confirmed. The field is
// part of the template, so a prompt and a yes-or-no are the same dialog with one row hidden.
function ask(word, label, danger, value) {
  return new Promise((done) => {
    const modal = build("tpl-dialog");
    text(modal, "text", word);
    const yes = modal.querySelector('[data-ref="yes"]');
    yes.textContent = label;
    if (danger) yes.className = "btn danger";
    const box = modal.querySelector('[data-ref="field"]');
    const asked = typeof value === "string";
    if (asked) {
      box.hidden = false;
      box.value = value;
    }
    const answer = () => {
      const said = asked ? box.value.trim() : true;
      modal.close();
      done(asked ? said || null : true);
    };
    yes.addEventListener("click", answer);
    if (asked) {
      box.addEventListener("keydown", (event) => {
        if (event.key !== "Enter") return;
        event.preventDefault();
        answer();
      });
    }
    modal.querySelector('[data-ref="no"]').addEventListener("click", () => modal.close());
    modal.addEventListener("close", () => {
      modal.remove();
      done(asked ? null : false);
    });
    document.body.append(modal);
    modal.showModal();
    if (asked) {
      box.focus();
      box.select();
    }
  });
}

export function confirm(word, label, danger = true) {
  return ask(word, label, danger, null);
}

// One line of text, held as the modal's own field: an empty answer is no answer.
export function typed(word, label, value = "") {
  return ask(word, label, false, value);
}

export function draw() {
  const list = clear(at("session-list"));
  if (!state.rows.length) {
    const empty = document.createElement("li");
    empty.className = "empty";
    empty.textContent = "no sessions yet";
    list.append(empty);
    return;
  }
  for (const row of state.rows) {
    const running = state.session && state.session.id === row.id && state.session.busy;
    const node = build("tpl-rail-item");
    node.dataset.id = String(row.id);
    node.dataset.on = row.id === state.active ? "1" : "0";
    node.dataset.live = running ? "1" : "0";
    node.addEventListener("click", () => enter(row.id));
    text(node, "title", title(row));
    text(node, "id", "#" + row.id);
    text(node, "rounds", row.rounds ? row.rounds + "r" : "");
    text(node, "time", row.last_ts ? short(String(row.last_ts).replace("T", " "), 16) : "");
    text(node, "live", running ? "running" : "");

    node.querySelector(".rename").addEventListener("click", async (event) => {
      event.stopPropagation();
      const said = await typed("name this session", "rename", row.name || "");
      if (said) rename(said, row.id);
    });
    node.querySelector(".remove").addEventListener("click", async (event) => {
      event.stopPropagation();
      const sure = await confirm("delete session #" + row.id + " for good?", "delete");
      if (sure) remove(row.id);
    });
    list.append(node);
  }
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
