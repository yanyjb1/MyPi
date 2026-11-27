// One question asked in a native modal, so a destructive write cannot be a slip.

import { el } from "../base/dom.js";

// A modal that answers with its own field's text, or with `true` when it only had to be confirmed.
function ask(text, label, danger, field) {
  return new Promise((done) => {
    const yes = el(
      "button",
      { class: danger ? "btn danger" : "btn primary", type: "button" },
      label,
    );
    const no = el("button", { class: "btn", type: "button" }, "cancel");
    const modal = el(
      "dialog",
      {},
      el("p", {}, text),
      field || null,
      yes,
      no,
    );
    const answer = () => {
      const said = field ? field.value.trim() : true;
      modal.close();
      done(field ? said || null : true);
    };
    yes.addEventListener("click", answer);
    if (field) {
      field.addEventListener("keydown", (event) => {
        if (event.key !== "Enter") return;
        event.preventDefault();
        answer();
      });
    }
    no.addEventListener("click", () => modal.close());
    modal.addEventListener("close", () => {
      modal.remove();
      done(field ? null : false);
    });
    document.body.append(modal);
    modal.showModal();
    if (field) {
      field.focus();
      field.select();
    }
  });
}

export function confirm(text, label, danger = true) {
  return ask(text, label, danger, null);
}

// One line of text, held as the modal's own field: an empty answer is no answer.
export function typed(text, label, value = "") {
  const field = el("input", { spellcheck: "false" });
  field.value = value || "";
  return ask(text, label, false, field);
}