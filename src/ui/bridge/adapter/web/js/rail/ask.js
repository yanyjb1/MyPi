// One question asked in a native modal, so a destructive write cannot be a slip.

export function confirm(text, label, danger = true) {
  return new Promise((done) => {
    const dialog = document.createElement("dialog");
    const yes = document.createElement("button");
    yes.className = danger ? "btn danger" : "btn primary";
    yes.textContent = label;
    const no = document.createElement("button");
    no.className = "btn";
    no.textContent = "cancel";
    const line = document.createElement("p");
    line.textContent = text;
    dialog.append(line, yes, no);
    yes.addEventListener("click", () => {
      dialog.close();
      done(true);
    });
    no.addEventListener("click", () => dialog.close());
    dialog.addEventListener("close", () => {
      dialog.remove();
      done(false);
    });
    document.body.append(dialog);
    dialog.showModal();
  });
}

// One line of text, held as the modal's own field: an empty answer is no answer.
export function typed(text, label, value = "") {
  return new Promise((done) => {
    const dialog = document.createElement("dialog");
    const field = document.createElement("input");
    field.value = value || "";
    field.spellcheck = false;
    const yes = document.createElement("button");
    yes.className = "btn primary";
    yes.textContent = label;
    const no = document.createElement("button");
    no.className = "btn";
    no.textContent = "cancel";
    const line = document.createElement("p");
    line.textContent = text;
    dialog.append(line, field, yes, no);
    const answer = () => {
      const said = field.value.trim();
      dialog.close();
      done(said || null);
    };
    yes.addEventListener("click", answer);
    field.addEventListener("keydown", (event) => {
      if (event.key !== "Enter") return;
      event.preventDefault();
      answer();
    });
    no.addEventListener("click", () => dialog.close());
    dialog.addEventListener("close", () => {
      dialog.remove();
      done(null);
    });
    document.body.append(dialog);
    dialog.showModal();
    field.focus();
    field.select();
  });
}
